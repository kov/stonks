use std::fmt::{self, Debug, Display};
use chrono::{Utc,Local,DateTime};
use mongodb::bson;
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
        match db.list_collection_names(bson::doc! {
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
        match collection.insert_one(bson::doc! {
            "kind": kind.to_string(),
            "quantity": quantity,
            "price": price.to_f64().unwrap(),
            "date": bson::Bson::DateTime(actual_date.with_timezone(&Utc)),
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
