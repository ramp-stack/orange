#![allow(clippy::new_ret_no_self)]

use std::sync::{Arc, Mutex};

use crate::wallet::{self, WalletService, WalletTx};

use chk::{
    FormItem, NumberVariant, Flow, Bumper,
    Display, Offset, Context, Page, PageType, Icons,
    Theme, Form, Root, State, Review, Success, FormSubmit, FormComplete, FormAction,
    Timestamp, TableItem, FormValidState,
};

#[derive(Debug, Clone)]
pub struct BitcoinHome;

impl BitcoinHome {
    pub fn new(wallet: &Arc<Mutex<WalletService>>) -> Root {
        let sw = wallet.clone();
        let rw = wallet.clone();
        let send = Box::new(move |_: &mut Context, theme: &Theme| Flow::from_form(SendForm::new(theme, &sw)));
        let receive = Box::new(move |_: &mut Context, _theme: &Theme| Flow::new(vec![Receive::new(&rw)]));

        let mut w = wallet.lock().unwrap();
        let price = w.price().unwrap();
        let balance = w.balance().unwrap();

        Root::new("Wallet",
            vec![
                Display::currency(move || balance.0.to_btc() * price, &balance.btc()),
            ],
            None, Some(("Receive".into(), receive)), Some(("Send".into(), send)),
        )
    }
}

pub struct Receive;
impl Receive {
    pub fn new(wallet: &Arc<Mutex<WalletService>>) -> Page {
        let address = wallet.lock().unwrap().next_address().expect("Could not get next address").to_qr_uri();
        Page::Static(PageType::display_qr_code("Receive bitcoin", &address, "Scan to receive bitcoin."))
    }
}

pub struct ViewTransaction;
impl ViewTransaction {
    pub fn new(price: f64, transaction: WalletTx) -> Page {
        let timestamp = Timestamp::new(transaction.timestamp.map(|dt| dt.into()));

        let items = match transaction.received {
            true => vec![
                TableItem::new("Date", &timestamp.date()),
                TableItem::new("Time", &timestamp.time()),
                TableItem::new("Received at address", &transaction.address_short.unwrap()),
                TableItem::new("Bitcoin received", &transaction.amount.btc()),
                TableItem::new("Bitcoin price", &wallet::Amount::usd_from_f32(transaction.btc_price_usd.unwrap() as f32)),
                TableItem::new("Amount received", &transaction.amount.usd(price))
            ],
            false => vec![
                TableItem::new("Date", &timestamp.date()),
                TableItem::new("Time", &timestamp.time()),
                TableItem::new("Sent to address", &transaction.address_short.unwrap_or_default()),
                TableItem::new("Bitcoin sent", &transaction.amount.btc()),
                TableItem::new("Bitcoin price", &wallet::Amount::usd_from_f32(transaction.btc_price_usd.unwrap() as f32)),
                TableItem::new("Amount sent", &transaction.amount.usd(price)),
                TableItem::new("Network fee", &transaction.fee.unwrap().usd(price)),
                TableItem::new("Total", &(transaction.fee.unwrap() + transaction.amount).usd(price))
            ]
        };

        Page::Static(PageType::display(
            &format!("{} bitcoin", if transaction.received {"Received"} else {"Sent"}),
            vec![
                Display::currency(move || transaction.amount.0.to_btc() * price, &transaction.amount.btc()),
                Display::table("Transaction details", items, None),
            ],
            None,
            Bumper::Done,
            Offset::Start,
        ))
    }
}

pub struct SendForm;
impl SendForm {
    pub fn new(theme: &Theme, wallet: &Arc<Mutex<WalletService>>) -> Form {
        let w = wallet.clone();
        let price = wallet.lock().unwrap().price().unwrap();
        let (low, high) = wallet.lock().unwrap().required().unwrap();

        let closure = Box::new(move |_: &mut Context, objects: &Vec<State>| {
            let State::Text(address) = objects[0].clone() else { panic!("No Address"); };
            let State::Number(amount_input) = objects[1].clone() else { panic!("No Amount"); };
            let State::Enumerator(priority) = objects[2].clone() else { panic!("No Priority"); };

            let usd = amount_input.trim_start_matches('$').parse::<f64>().unwrap_or_default();
            let amount_btc = bitcoin::Amount::from_sat(((usd / price) * 100_000_000.0).round() as u64);

            let fee_rate = match priority { 0 => 2, _ => 5 };
            let result = w.clone().lock().unwrap().send_to_address(&address, amount_btc.to_sat(), fee_rate);

            match result {
                Ok(txid) => println!("broadcasted tx: {txid}"),
                Err(err) => eprintln!("send failed: {err}"),
            }

            FormComplete::None
        }) as Box<dyn FormSubmit>;

        let w = wallet.clone();
        let review = move |objects: &Vec<State>| {
            let State::Text(address) = objects[0].clone() else { panic!("No Address"); };
            let State::Number(amount_input) = objects[1].clone() else { panic!("No Amount"); };
            let State::Enumerator(priority) = objects[2].clone() else { panic!("No Priority"); };

            let usd = amount_input.trim_start_matches('$').parse::<f64>().unwrap_or_default();
            let amount = wallet::Amount::new(bitcoin::Amount::from_sat(((usd / price) * 100_000_000.0).round() as u64));

            let (low, high) = w.clone().lock().unwrap().estimate_fees(address.to_string(), amount).unwrap();

            let (speed_label, fee) = match priority {
                1 => ("Priority (~30 minutes)", high),
                _ => ("Standard (~2 hours)", low),
            };

            vec![
                Display::cta("Confirm address", Some(&address), "Bitcoin sent to the wrong address can never be recovered.", vec![]),
                Display::table("Confirm amount", vec![
                    TableItem::new("Bitcoin sent", &amount.btc()),
                    TableItem::new("Send speed", speed_label),
                    TableItem::new("Amount sent", &amount.usd(price)),
                    TableItem::new("Transaction fee", &fee.usd(price)),
                    TableItem::new("Transaction total", &(amount + fee).usd(price)),
                ], None),
            ]
        };

        let success = |objects: Vec<State>| {
            let amount = if let State::Number(x) = &objects[1] {x} else {"$0.00"};
            let usd = amount.trim_start_matches('$').parse::<f64>().unwrap_or_default();
            (Icons::Bitcoin, format!("You sent ${:.2}", usd))
        };

        let w = wallet.clone();
        Form::flow(theme, vec![
            FormItem::text("Bitcoin address", "Enter a bitcoin address", Some(vec![
                FormAction::Paste, FormAction::scan_qr_code("Scan a bitcoin QR code"),
            ]), |_ctx: &mut Context, a: String| FormValidState::from(WalletService::ui_valid_address(&a))),
            FormItem::number("Bitcoin amount", NumberVariant::Currency, move |_ctx: &mut Context, a: String| FormValidState::from(w.clone().lock().unwrap().ui_can_afford(a))),
            FormItem::enumerator("Transaction speed", vec![
                ("Standard", &format!("Arrives in ~2 hours\n{} bitcoin network fee", low.usd(price))),
                ("Priority", &format!("Arrives in ~30 minutes\n{} bitcoin network fee", high.usd(price))),
            ]),
        ], Some(Review::new("Confirm send", review)), Some(Success::new("Bitcoin sent", success)), closure)
    }
}
