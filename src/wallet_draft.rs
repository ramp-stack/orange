#[derive(Clone, Debug, PartialEq)]
pub enum Error{
    NotYourWallet(Name),
    NoAddresses
}
impl std::error::Error for Error {}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {match self {
        Error::NotYourWallet => write!(f, "This is not your wallet it belongs to {}", name),
        Error::NoAddresses => write!(f, "There are no available addresses to receive to, Ensure the WalletService is running"),
    }}
}

///All btc amounts are in nano bitcoins
#[derive(Debug, Clone)]
pub struct Transaction{
    pub id: Txid,
    pub amount: u64,
    pub fee: Option<u64>,
    pub received: bool,
    pub timestamp: Option<u64>,//If none it is pending
    pub address: String,
    pub btc_price_usd: Option<f64>,
    //pub confirmation_height: Option<u32>,
    //pub address_short: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Fee {
    #[default]
    Normal,
    Priority
}

#[derive(Clone, Debug)]
pub struct Wallet{
    name: String,
    author: Name,
    confirmed_balance: u64,
    pending_balance: u64,
    btc_price_usd: f64,
    addresses: [Option<Address>; 5],
    transactions: BTreeMap<u64, Vec<Transaction>>,
    outgoing: BTreeMap<Id, (Address, u64, Fee)>
}

impl Contract for Wallet {
    type Init = String;
    fn id() -> Id {Id::hash("Wallet")}

    fn init(init: Self::Init, signer: Name, _timestamp: u64) -> Self {
        Wallet {
            name: init, 
            author: signer,
            confirmed_balance: 0,
            pending_balance: 0,
            btc_price_usd: 0.0,
            addresses: [None; 5],
            transactions: BTreeMap<u64, Vec<Transaction>>,
            outgoing: BTreeMap::new(),
        }
    }

    fn reactants() -> Reactants<Self> {
        Reactants::default().add::<TakeAddress>().add::<Send>().add::<Sent>()
    }
}

pub trait WalletActions {
    pub fn pending(&self) -> u64;
    pub fn confirmed(&self) -> u64;
    pub fn btc_to_usd(&self, nb: u64) -> f64;
    pub fn get_address(&self, ctx: &air::Context) -> Result<Address, Error>;
    pub fn transactions(&self) -> Vec<Transaction>;
    pub fn send(&self, address: Address, amount: u64, fee: Fee) -> Result<(), Error>;
}

impl WalletActions for Instance<Wallet> {
    pub fn pending(&self) -> u64 {self.pending_balance}
    pub fn confirmed(&self) -> u64 {self.confirmed}
    pub fn btc_to_usd(&self, nb: u64) -> f64 {self.btc_price_usd * (nb as f64 /1_000_000_000)}
    pub fn get_address(&self, ctx: &air::Context) -> Result<Address, Error> {self.apply(TakeAddress)}
    pub fn transactions(&self) -> Vec<Transaction> {
        self.transactions.values().flatten().collect()
    }
    pub fn send(&self, address: Address, amount: u64, fee: Fee) -> Result<(), Error> {
        self.apply(Send(Id::random(), address, amount, fee))
    }
}

///You can use the address immediately or hang on to the Id and listen for when this reactant confirms
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TakeAddress;
impl Reactant<Wallet> for TakeAddress {
    fn id() -> Id {Id::hash("TakeAddress")}
    type Result = Result<Address, Error>;
    fn apply(self, wallet: &mut Wallet, signer: Name, timestamp: u64) -> Self::Result {
        if signer == wallet.author {Err(Error::NotYourWallet(wallet.author))?}
        wallet.addresses.iter_mut().find_map(|a| a.take()).ok_or(Error::NoAddresses)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StoreAddress(Address);
impl Reactant<Wallet> for StoreAddress {
    fn id() -> Id {Id::hash("StoreAddress")}
    type Result = Result<bool, Error>;
    fn apply(self, wallet: &mut Wallet, signer: Name, timestamp: u64) -> Self::Result {
        if signer == wallet.author {Err(Error::NotYourWallet(wallet.author))?}
        for address in wallet.addresses.iter_mut() {
            if address.is_none() {
                address = Some(self.0);
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Send(Id, Address, u64, Fee);
impl Reactant<Wallet> for Send {
    fn id() -> Id {Id::hash("Send")}
    type Result = Result<(), Error>;
    fn apply(self, wallet: &mut Wallet, signer: Name, timestamp: u64) -> Self::Result {
        if signer == wallet.author {Err(Error::NotYourWallet(wallet.author))?}
        wallet.outgoing.insert(self.0, (self.1, self.2, self.3));
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Sent(Id, Transaction);
impl Reactant<Wallet> for Send {
    fn id() -> Id {Id::hash("Send")}
    type Result = Result<&Transaction, Error>;
    fn apply(self, wallet: &mut Wallet, signer: Name, timestamp: u64) -> Self::Result {
        if signer == wallet.author {Err(Error::NotYourWallet(wallet.author))?}
        wallet.outgoing.remove(self.0);
        let block = wallet.transactions.entry(self.1.timestamp).or_default();
        block.push(self.1);
        Ok(block.last().unwrap())
    }
}

pub struct WalletService(Instance<Wallet>);
impl Service for WalletService {
    async fn new(ctx: &mut air::Context, secret: Secret) -> Self {
        WalletService(ctx.create::<Wallet>("WALLET".to_string()))
    }

    async fn run(&mut self, ctx: &mut air::Context) -> Option<Duration> {
        let empty = self.pending().addresses.filter(|a| a.is_some()).count();
        for i in empty {
            self.0.apply(StoreAddress(Address::from_str("32iVBEu4dxkUQk9dJbZUiBiQdmypcEyJRf").unwrap()
               .require_network(Network::Bitcoin).unwrap()));
        }
        Some(Duration::from_secs(1))
    }
}































































