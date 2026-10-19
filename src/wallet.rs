#![allow(dead_code)]

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::{fs, env};
use std::path::PathBuf;
use std::str::FromStr;
use chrono::{DateTime, Utc};

use bdk_wallet::{
    serde_json,
    descriptor::template::Bip86,
    bitcoin::{
        bip32::Xpriv,
        secp256k1,
        Address, FeeRate, Network, Txid,
    },
    chain::{Merge, ChainPosition, Anchor},
    ChangeSet, KeychainKind, Wallet, WalletPersister, PersistedWallet,
};

use bitcoin::address::NetworkUnchecked;

use bdk_wallet::chain::spk_client::SyncRequest;
use bdk_wallet::chain::keychain_txout::SyncRequestBuilderExt;

use bdk_esplora::{
    esplora_client::{BlockingClient, Builder as EsploraBuilder},
    EsploraExt,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePersister {
    path: PathBuf,
    changeset: ChangeSet,
}

impl Default for FilePersister {
    fn default() -> Self { Self::new() }
}

impl FilePersister {
    pub fn new() -> Self {
        let path = env::current_dir().expect("Failed to get current dir").join("wallet_state.json");

        Self { changeset: match path.exists() {
            true => serde_json::from_slice(&fs::read(&path).unwrap_or_default()).unwrap_or_default(),
            false => ChangeSet::default()
        }, path }
    }

    fn save(&self) -> Result<()> {
        let data = serde_json::to_vec_pretty(&self.changeset)?;
        fs::write(&self.path, data)?;
        Ok(())
    }

    pub fn from_changeset(changeset: ChangeSet) -> Self {
        Self {
            path: PathBuf::from("/tmp/wallet_state_estimate.json"),
            changeset,
        }
    }
}

impl WalletPersister for FilePersister {
    type Error = anyhow::Error;

    fn initialize(p: &mut Self) -> std::result::Result<ChangeSet, Self::Error> {
        Ok(p.changeset.clone())
    }

    fn persist(p: &mut Self, changeset: &ChangeSet) -> std::result::Result<(), Self::Error> {
        p.changeset.merge(changeset.clone());
        p.save()?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct WalletTx {
    pub txid: Txid,
    pub amount: Amount,
    pub received: bool,
    pub confirmed: bool,
    pub confirmation_height: Option<u32>,
    pub timestamp: Option<DateTime<Utc>>,
    pub address: Option<String>,
    pub address_short: Option<String>,
    pub btc_price_usd: Option<f64>,
    pub fee: Option<Amount>,
}

pub struct WalletService {
    pub wallet: PersistedWallet<FilePersister>,
    persister: FilePersister,
    esplora: BlockingClient,
}

impl WalletService {
    pub fn new() -> Result<Self> {
        let xpriv = Self::load_or_create_xpriv()?;
        let mut persister = FilePersister::new();

        let ext = Bip86(xpriv, KeychainKind::External);
        let int = Bip86(xpriv, KeychainKind::Internal);

        let loaded = Wallet::load()
            .descriptor(KeychainKind::External, Some(ext.clone()))
            .descriptor(KeychainKind::Internal, Some(int.clone()))
            .extract_keys()
            .check_network(Network::Bitcoin)
            .load_wallet(&mut persister)?;

        let esplora = EsploraBuilder::new("https://blockstream.info/api").build_blocking();

        Ok(match loaded {
            Some(wallet) => {
                let mut wallet = Self { wallet, esplora, persister };
                wallet.sync()?;
                wallet
            },
            None => {
                let wallet = Wallet::create(ext, int)
                    .network(Network::Bitcoin)
                    .create_wallet(&mut persister)
                    .expect("wallet create failed");

                let mut wallet = Self { wallet, esplora, persister };
                wallet.full_sync()?;
                wallet
            }
        })
    }

    fn load_or_create_xpriv() -> Result<Xpriv> {
        let path = env::current_dir().expect("Failed to get current dir").join("wallet_xpriv");
        if path.exists() { return Ok(Xpriv::from_str(fs::read_to_string(&path)?.trim())?); }
        let secret = secp256k1::SecretKey::new(&mut secp256k1::rand::thread_rng());
        let xpriv = Xpriv::new_master(Network::Bitcoin, &secret.secret_bytes())?;
        fs::write(&path, xpriv.to_string())?;
        Ok(xpriv)
    }

    pub fn full_sync(&mut self) -> Result<()> {
        let req = self.wallet.start_full_scan().build();
        let update = self.esplora.full_scan(req, 20, 4)?;
        self.wallet.apply_update(update)?;
        self.wallet.persist(&mut self.persister)?;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        let req = SyncRequest::builder()
            .chain_tip(self.wallet.local_chain().tip())
            .revealed_spks_from_indexer(self.wallet.spk_index(), ..)
            .build();

        let update = self.esplora.sync(req, 12)?;
        self.wallet.apply_update(update)?;
        self.wallet.persist(&mut self.persister)?;
        Ok(())
    }

    pub fn balance(&mut self) -> Result<Amount> {
        Ok(Amount::new(self.wallet.balance().total()))
    }

    pub fn next_address(&mut self) -> Result<Address> {
        let address = self.wallet.next_unused_address(KeychainKind::External).address;
        self.wallet.persist(&mut self.persister)?;
        Ok(address)
    }

    pub fn validate_address(s: &str) -> Result<Address> {
        let uri = unified_uri::UnifiedUri::from_str(s).map_err(|_| anyhow!("Invalid str while parsing address"))?;
        let address: Address<NetworkUnchecked> = uri.address().clone();
        address.require_network(Network::Bitcoin).map_err(|_| anyhow!("wrong network"))
    }

    pub fn price(&self) -> Result<f64> { 
        let url = "https://api.coinbase.com/v2/prices/BTC-USD/spot";
        let body = ureq::get(url).call()?.into_string()?;
        let json: serde_json::Value = serde_json::from_str(&body)?;
        let price_str = json["data"]["amount"].as_str().ok_or_else(|| anyhow!("missing price field"))?;
        Ok(price_str.parse::<f64>()?)
    }

    fn btc_price_usd_at(timestamp: DateTime<Utc>) -> Result<f64> {
        let start = timestamp.format("%Y-%m-%dT%H:%M:00Z");
        let end = (timestamp + chrono::Duration::minutes(1)).format("%Y-%m-%dT%H:%M:00Z");
        let url = format!("https://api.exchange.coinbase.com/products/BTC-USD/candles?granularity=60&start={start}&end={end}");
        let body = ureq::get(&url).call()?.into_string()?;
        let data: Vec<Vec<f64>> = serde_json::from_str(&body)?;
        let candle = data.first().ok_or_else(|| anyhow!("no candle data"))?;
        Ok(candle[4])
    }

    fn abbreviate_address(addr: &str) -> String {
        match addr.len() <= 13 {
            true => addr.to_string(),
            false => format!("{}...{}", &addr[..7], &addr[addr.len() - 3..])
        }
    }

    fn tx_address(&self, tx: &bdk_wallet::bitcoin::Transaction, received: bool) -> Option<String> {
        let output = match received {
            true => tx.output.iter().find(|o| self.wallet.is_mine(o.script_pubkey.clone())),
            false => tx.output.iter().find(|o| !self.wallet.is_mine(o.script_pubkey.clone())),
        }?;

        Address::from_script(&output.script_pubkey, Network::Bitcoin).ok().map(|a| a.to_string())
    }

    fn tx_timestamp(&self, chain_position: &ChainPosition<impl Anchor>) -> Option<DateTime<Utc>> {
        match chain_position {
            ChainPosition::Confirmed { anchor, .. } => {
                let block_hash = anchor.anchor_block().hash;
                let url = format!("https://mempool.space/api/block/{block_hash}");

                let ts = ureq::get(&url).call().ok()?.into_string().ok()
                    .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
                    .and_then(|json| json["timestamp"].as_i64())?;

                DateTime::<Utc>::from_timestamp(ts, 0)
            }
            ChainPosition::Unconfirmed { .. } => None,
        }
    }

    fn temp_wallet(&self) -> Result<PersistedWallet<FilePersister>> {
        let xpriv = Self::load_or_create_xpriv()?;
        let ext = Bip86(xpriv, KeychainKind::External);
        let int = Bip86(xpriv, KeychainKind::Internal);

        let mut persister = FilePersister::from_changeset(self.persister.changeset.clone());

        Wallet::load()
            .descriptor(KeychainKind::External, Some(ext.clone()))
            .descriptor(KeychainKind::Internal, Some(int.clone()))
            .extract_keys()
            .check_network(self.wallet.network())
            .load_wallet(&mut persister)?
            .ok_or_else(|| anyhow!("failed to load temp wallet"))
    }

    pub fn estimate_fees(&self, address: String, amount: Amount) -> Result<(Amount, Amount)> {
        let amount: bitcoin::Amount = amount.0;
        let to = Self::validate_address(&address)?;

        let standard = FeeRate::from_sat_per_vb(3).ok_or_else(|| anyhow!("invalid standard feerate"))?;
        let priority = FeeRate::from_sat_per_vb(5).ok_or_else(|| anyhow!("invalid priority feerate"))?;

        let fee_for = |fee_rate: FeeRate| -> Result<bitcoin::Amount> {
            let mut wallet = self.temp_wallet()?;
            let mut builder = wallet.build_tx();
            builder.add_recipient(to.script_pubkey(), amount).fee_rate(fee_rate);
            let psbt = builder.finish()?;
            let input_sum: u64 = psbt.inputs.iter().filter_map(|i| i.witness_utxo.as_ref()).map(|u| u.value.to_sat()).sum();
            let output_sum: u64 = psbt.unsigned_tx.output.iter().map(|o| o.value.to_sat()).sum();

            Ok(bitcoin::Amount::from_sat(input_sum.saturating_sub(output_sum)))
        };

        Ok((Amount::new(fee_for(standard)?), Amount::new(fee_for(priority)?)))
    }

    pub fn required(&mut self) -> Result<(Amount, Amount)> {
        let estimated_vbytes = 140; 
        let fee_rate = FeeRate::from_sat_per_vb(3).ok_or_else(|| anyhow!("invalid feerate"))?;
        let low = estimated_vbytes * fee_rate.to_sat_per_vb_floor();
        let fee_rate = FeeRate::from_sat_per_vb(5).ok_or_else(|| anyhow!("invalid feerate"))?;
        let high = estimated_vbytes * fee_rate.to_sat_per_vb_floor();
        Ok((Amount::new(bitcoin::Amount::from_sat(low)), Amount::new(bitcoin::Amount::from_sat(high))))
    }

    pub fn send(&mut self, to: Address, amount: bitcoin::Amount, fee_rate: FeeRate) -> Result<Txid> {
        let mut builder = self.wallet.build_tx();
        builder.add_recipient(to.script_pubkey(), amount).fee_rate(fee_rate);

        let mut psbt = builder.finish()?;
        self.wallet.persist(&mut self.persister)?;
        if !self.wallet.sign(&mut psbt, bdk_wallet::SignOptions::default())? {
            return Err(anyhow!("tx not finalized"));
        }

        let tx = psbt.extract_tx()?;
        let txid = tx.compute_txid();
        // for (i, output) in tx.output.iter().enumerate() {
        //     let addr = Address::from_script(&output.script_pubkey, self.network).ok();
        //     let mine = self.wallet.is_mine(output.script_pubkey.clone());
        // }

        self.esplora.broadcast(&tx)?;
        Ok(txid)
    }

    pub fn send_to_address(&mut self, to: &str, amount_sat: u64, fee_rate_sat_vb: u64) -> Result<Txid> {
        let address = Self::validate_address(to)?;
        let amount = bitcoin::Amount::from_sat(amount_sat);
        let fee_rate = FeeRate::from_sat_per_vb(fee_rate_sat_vb).ok_or_else(|| anyhow!("invalid feerate"))?;
        let txid = self.send(address, amount, fee_rate)?;

        Ok(txid)
    }

    pub fn ui_can_afford(&mut self, usd: String) -> Result<String, String> {
        let usd = usd.trim_start_matches('$').replace(',', "").parse::<f64>().unwrap_or_default();
        let balance = self.balance().unwrap().0;
        let (_low, high) = self.required().unwrap();
        let price = self.price().unwrap();
        let amount = bitcoin::Amount::from_sat(((usd / price) * 100_000_000.0).round() as u64);
        let required_btc = high.0 + amount;
        let minimum = required_btc - amount; // just the fees
        match usd <= 0.0 {
            true => Err(String::new()),
            false if required_btc > balance => Err(format!("Maximum send {}", Amount::new(balance - minimum).usd(price))),
            false if minimum > amount => Err(format!("Minimum send {}", Amount::new(minimum).usd(price))),
            false => Ok(format!("{:.8} BTC", amount.to_btc()))
        }
    }

    pub fn ui_valid_address(address: &str) -> Result<String, String> {
        match address.is_empty() {
            true => Err(String::new()),
            false => Self::validate_address(address).map(|_| String::new()).map_err(|_| "This is not a valid bitcoin address.".to_string()),
        }
    }

    pub fn transactions(&mut self) -> Result<Vec<WalletTx>> {
        let mut txs: Vec<_> = self.wallet.transactions().map(|t| {
            let tx = &t.tx_node.tx;
            let (sent, recv) = self.wallet.sent_and_received(tx);
            let timestamp = self.tx_timestamp(&t.chain_position);
            let received = recv.to_sat() >= sent.to_sat();
            let address = self.tx_address(tx, received);
            let fee = self.wallet.calculate_fee(tx).ok().unwrap_or_default();
            let mut sent = recv.to_sat().abs_diff(sent.to_sat());
            if !received { sent -= fee.to_sat(); }


            WalletTx {
                txid: t.tx_node.txid,
                fee: (!received).then_some(Amount::new(fee)),
                amount: Amount::new(bitcoin::Amount::from_sat(sent)),
                confirmed: t.chain_position.is_confirmed(),
                confirmation_height: t.chain_position.confirmation_height_upper_bound(),
                address_short: address.as_deref().map(Self::abbreviate_address),
                btc_price_usd: timestamp.and_then(|ts| Self::btc_price_usd_at(ts).ok()),
                timestamp,
                address,
                received,
            }
        }).collect();

        txs.sort_by(|a, b| {
            match (&a.timestamp, &b.timestamp) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Less, 
                (Some(_), None) => std::cmp::Ordering::Greater,
                (Some(a_ts), Some(b_ts)) => b_ts.cmp(a_ts),
            }
        });

        Ok(txs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Amount(pub bitcoin::Amount);

impl Amount {
    pub fn new(amt: bitcoin::Amount) -> Self {Amount(amt)}

    pub fn usd(&self, price: f64) -> String {
        let value = self.0.to_btc() * price;
        Self::usd_from_f32(value as f32)
    }

    pub fn usd_from_f32(v: f32) -> String {
        let (mut d, mut c) = (v.abs().trunc() as u64, (v.abs().fract() * 100.0).round() as u64);
        if c == 100 { d += 1; c = 0; }
        let mut s = d.to_string();
        for i in (1..s.len().div_ceil(3)).rev() { s.insert(s.len()-3*i, ','); }
        format!("{}${}.{:02}", if v < 0.0 { "-" } else { "" }, s, c)
    }

    pub fn usd_f32(&self, price: f64) -> f32 { (self.0.to_btc() * price) as f32 }
    pub fn btc(&self) -> String { format!("{:.8} BTC", self.0.to_btc()) }
    // pub fn btc_f64(&self) -> f64 { self.0.to_btc() }
    // pub fn from_btc(amt: f64) -> Self { Amount(bitcoin::Amount::from_btc(((amt * 100_000_000.0).round() as u64) as f64).unwrap()) }
}

use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub, SubAssign};

impl Add for Amount {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Amount(bitcoin::Amount::from_sat(self.0.to_sat() + rhs.0.to_sat()))
    }
}

impl AddAssign for Amount {
    fn add_assign(&mut self, rhs: Self) {
        self.0 = bitcoin::Amount::from_sat(self.0.to_sat() + rhs.0.to_sat());
    }
}

impl Sub for Amount {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Amount(bitcoin::Amount::from_sat(self.0.to_sat().saturating_sub(rhs.0.to_sat())))
    }
}

impl SubAssign for Amount {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 = bitcoin::Amount::from_sat(self.0.to_sat().saturating_sub(rhs.0.to_sat()));
    }
}

impl Mul<u64> for Amount {
    type Output = Self;

    fn mul(self, rhs: u64) -> Self::Output {
        Amount(bitcoin::Amount::from_sat(self.0.to_sat().saturating_mul(rhs)))
    }
}

impl MulAssign<u64> for Amount {
    fn mul_assign(&mut self, rhs: u64) {
        self.0 = bitcoin::Amount::from_sat(self.0.to_sat().saturating_mul(rhs));
    }
}

impl Div<u64> for Amount {
    type Output = Self;

    fn div(self, rhs: u64) -> Self::Output {
        Amount(bitcoin::Amount::from_sat(self.0.to_sat() / rhs))
    }
}

impl DivAssign<u64> for Amount {
    fn div_assign(&mut self, rhs: u64) {
        self.0 = bitcoin::Amount::from_sat(self.0.to_sat() / rhs);
    }
}
