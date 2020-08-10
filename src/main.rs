use std::fmt::{self, Debug, Display};
use chrono::{Utc,Local,DateTime};
use mongodb::bson::{Bson, doc};
use rust_decimal::prelude::*;
use rustyline::error::ReadlineError;
use rustyline::Editor;
use structopt::clap::AppSettings;
use structopt::StructOpt;

#[derive(StructOpt)]
#[structopt(
    global_settings = &[AppSettings::NoBinaryName]
)]
struct Statement {
    #[structopt(subcommand)]
    command: Option<Command>
}

#[derive(Debug, StructOpt)]
enum Command {
    Ls {
        filter: Option<String>,
    },

    Buy {
        ticker: String,
        quantity: i64,
        price: Decimal,
        date: Option<DateTime<Local>>,
    },

    Sell {
        ticker: String,
        quantity: i64,
        price: Decimal,
        date: Option<DateTime<Local>>,
    },

    AvgPrice {
        filter: Option<String>,
        until: Option<DateTime<Local>>,
    }
}

#[derive(Debug)]
enum OperationKind {
    Buy,
    Sell,
}

impl Display for OperationKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Debug::fmt(self, f)
    }
}

impl FromStr for OperationKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_lowercase();
        match lower.as_str() {
            "buy" => Ok(OperationKind::Buy),
            "sell" => Ok(OperationKind::Sell),
            _ => Err(String::from("Unknown operation kind"))
        }
    }
}

struct Position {
    ticker: String,
    value: f64,
    quantity: i64,
    average_price: f64,
}

struct App {
    editor: Editor<()>,
    db_client: mongodb::sync::Client,
}

impl App {
    pub fn new() -> App {
        let db_client = mongodb::sync::Client::with_uri_str("mongodb://127.0.0.1:27017/")
            .expect("Could not connect to mongodb");
        App {
            editor: Editor::<()>::new(),
            db_client: db_client,
        }
    }

    pub fn tokenize_line(&self, line: &str) -> Vec<String> {
        let mut parts = Vec::<String>::new();
        let mut current = String::new();
        let mut in_quotes = false;
        for c in line.chars() {
            match c {
                ' ' => {
                    if in_quotes {
                        current.push(c);
                        continue;
                    }
                    parts.push(current.to_string());
                    current.clear();
                },
                '"' => {
                    in_quotes = !in_quotes;
                },
                _ => {
                    current.push(c);
                }
            }
        }
        if in_quotes {
            println!("Unmatched quote");
        }
        if !current.is_empty() {
            parts.push(current.to_string());
        }
        println!("{:?}", parts);
        return parts;
    }

    pub fn parse_line(&self, line: &str) -> Option<Statement> {
        let tokens = self.tokenize_line(line);
        let statement = match Statement::from_iter_safe(tokens) {
            Ok(statement) => statement,
            Err(e) => { println!("{}", e); return None }
        };
        match &statement.command {
            Some(_command) => Some(statement),
            None => None
        }
    }

    pub fn cmd_list(&self, filter: &Option<String>) {
        let db = self.db_client.database("stonks");
        match db.list_collection_names(doc! {
            "name": { "$regex": filter.as_ref().unwrap_or(&String::from("")) }
        }) {
            Ok(names) => {
                for name in names {
                    let collection = db.collection(name.as_str());
                    println!("{} ({} operations)",
                        name, collection.estimated_document_count(None).unwrap());
                }
            },
            Err(_) => (),
        }
        println!("ls filter: {:?}", filter);
    }

    pub fn add_operation(&self, kind: OperationKind,
        ticker: &String, quantity: &i64, price: &Decimal, date: &Option<DateTime<Local>>) {
        // Default to today if no date was given.
        let now = &chrono::Local::now();
        let actual_date = match date {
            Some(date) => date,
            None => now,
        };
        let collection = self.db_client.database("stonks").collection(ticker);
        match collection.insert_one(doc! {
            "kind": kind.to_string(),
            "quantity": quantity,
            "price": price.to_f64().unwrap(),
            "date": Bson::DateTime(actual_date.with_timezone(&Utc)),
        }, None) {
            Ok(_) => (),
            Err(e) => println!("Error inserting operation {}", e)
        }
        println!("{:?} {} {} {} {:?}", kind, ticker, quantity, price, date);
    }

    pub fn cmd_buy(&self, ticker: &String, quantity: &i64, price: &Decimal, date: &Option<DateTime<Local>>) {
        self.add_operation(OperationKind::Buy, ticker, quantity, price, date);
    }

    pub fn cmd_sell(&self, ticker: &String, quantity: &i64, price: &Decimal, date: &Option<DateTime<Local>>) {
        self.add_operation(OperationKind::Sell, ticker, quantity, price, date);
    }

    pub fn cmd_avgprice(&self, filter: &Option<String>, until: &Option<DateTime<Local>>) {
        let collection = self.db_client.database("stonks").collection("stocks");

        let mut pipeline = Vec::new();
        if let Some(f) = filter {
            pipeline.push(doc!{
                "$match" : { "ticker": { "$regex": &f } }
            });
        }
        pipeline.push(doc! {
                "$group" : { "_id": "$ticker" }
            }
        );
        let cursor = match collection.aggregate(pipeline, None) {
            Ok(cursor) => cursor,
            Err(e) => { println!("{}", e); return }
        };

        let mut tickers = Vec::new();
        for doc in cursor {
            match doc {
                Ok(doc) => {
                    tickers.push(doc.get_str("_id").unwrap().to_string());
                },
                Err(_) => ()
            }
        }
        for ticker in tickers {
            let average = self.average_price(&ticker, until);
            println!("{}\t{:>9.2}", &ticker, average)
        }
    }

    pub fn calculate_position(&self, ticker: &str, until: &Option<DateTime<Local>>) -> Position {
        let now = &chrono::Local::now();
        let date = match until {
            Some(date) => date,
            None => now,
        };

        let collection = self.db_client.database("stonks").collection("stocks");
        let filter = doc!{
            "$and": [
                { "ticker": &ticker },
                {
                    "date": {
                        "$lte": Bson::DateTime(date.with_timezone(&Utc))
                    }
                }
            ]
        };
        let cursor = match collection.find(filter, None) {
            Ok(cursor) => cursor,
            Err(e) => {
                println!("{}", e);
                return Position {
                    ticker: ticker.to_string(),
                    value: 0.0,
                    quantity: 0,
                    average_price: 0.0
                }
            }
        };

        let mut total_amount = 0f64;
        let mut total_quantity = 0i64;

        for document in cursor {
            if let Ok(document) = document {
                let quantity = document.get_i64("quantity").unwrap();
                let kind = OperationKind::from_str(document.get_str("kind").unwrap()).unwrap();
                match kind {
                    OperationKind::Buy => {
                        let price = document.get_f64("price").unwrap();
                        total_amount += price * quantity as f64;
                        total_quantity += quantity;
                    },
                    OperationKind::Sell => {
                        /* When selling, we need to use the average price of the buys
                         * at the moment for the average calculation to work. We may
                         * take out too little if the current price is lower or too
                         * much otherwise.
                         */
                        let price = total_amount / total_quantity as f64;
                        total_amount -= price * quantity as f64;
                        total_quantity -= quantity;
                    }
                }
            }
        }

        let average;
        if total_quantity == 0 || total_amount == 0.0 {
            average = 0.0;
        } else {
            average = total_amount / total_quantity as f64;
        }

        Position {
            ticker: ticker.to_string(),
            value: total_amount,
            quantity: total_quantity,
            average_price: average,
        }
    }

    pub fn average_price(&self, ticker: &str, until: &Option<DateTime<Local>>) -> f64 {
        let position = self.calculate_position(ticker, until);
        return position.average_price;
    }

    pub fn process_statement(&self, statement: Statement) {
        let command = statement.command.unwrap();
        match &command {
            Command::Ls { filter } => self.cmd_list(filter),
            Command::Buy { ticker, quantity, price, date } => {
                self.cmd_buy(ticker, quantity, price, date);
            },
            Command::Sell { ticker, quantity, price, date } => {
                self.cmd_sell(ticker, quantity, price, date);
            },
            Command::AvgPrice { filter, until } => {
                self.cmd_avgprice(filter, until);
            }
        }
    }

    pub fn run(&mut self) {
        // If arguments were given, execute the command and quit.
        let args: Vec<String> = std::env::args().collect();
        if args.len() > 1 {
            match Statement::from_iter_safe(&args[1..]) {
                Ok(statement) => { self.process_statement(statement); return },
                Err(e) => { println!("{}", e); return }
            }
        }

        loop {
            let readline = self.editor.readline(">> ");
            match readline {
                Ok(line) => {
                    self.editor.add_history_entry(line.as_str());
                    match self.parse_line(line.as_str()) {
                        Some(statement) => {
                            self.process_statement(statement)
                        },
                        None => (),
                    }
                },
                Err(ReadlineError::Interrupted) => {
                    break
                },
                Err(ReadlineError::Eof) => {
                    break
                },
                Err(err) => {
                    println!("Error: {:?}", err);
                    break
                }
            }
        }
    }
}

fn main() {
    let mut app = App::new();
    app.run()
}
