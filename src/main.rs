use std::net::IpAddr;
use serde_json::json;
use serde_derive::{Deserialize, Serialize};
use mailin_embedded::{Server, SslConfig, Handler, Response};
use mailin_embedded::response::OK;
use mailparse::{ParsedMail, MailHeaderMap};
use clap::Parser;

#[derive(Deserialize, Serialize, Clone, Debug)]
struct Account {
    address: String,
    telegram_bot_key: String,
    telegram_chat_id: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct SmtpServer {
    enabled: bool,
    hostname: String,
    address: String,
    port: i64,
    starttls: bool,
    cert_path: String,
    key_path: String,
    ca_path: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct ConfigFile {
    pub smtpservers: Vec<SmtpServer>,
    pub accounts: Vec<Account>,
}

#[derive(Parser)]
struct Cli {
    /// Sets a custom config file
    #[arg(value_name = "FILE", required = true )]
    pub config: String,
}

#[derive(Clone, Debug)]
struct MyHandler {
    mime: Vec<String>,
    accounts: Vec<Account>,
}

impl MyHandler {
    pub fn new(accounts: Vec<Account>) -> MyHandler {
        MyHandler { 
            mime: vec![],
            accounts: accounts
        }
    }
}

impl Handler for MyHandler {
    fn mail(
        &mut self, 
        _ip: IpAddr, 
        _domain: &str, 
        _from: &str
    ) -> Response {
        // hack add from header
        let from = "From: ".to_owned() + _from + "\n";
        self.mime.push(from.to_string());
        OK
    }

    fn data(
        &mut self, 
        buf: &[u8]
    ) -> std::io::Result<()> {
        // push data into msg
        self.mime.push(String::from_utf8(Vec::from(buf)).unwrap());
        Ok(())
    }

    fn data_end(&mut self) -> Response {
        // retrive mail
        let mime = self.mime.join("");

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let parsed = match mailparse::parse_mail(mime.as_ref()) {
                    Ok(p) => p,
                    Err(e) => return Err(e) // error parsing email
                };

                // check if destination address is defined
                let message = match parsed.get_headers().get_first_value("To") {
                    Some(acc) => {
                        let mut dest_found = false;

                        // destination is defined, check if destination account is on this server
                        for tg_account in &self.accounts {
                            if &tg_account.address.to_string() == &acc {
                                // destination account exists
                                // send telegram message
                                send_to_telegram(
                                    &parsed,
                                    &tg_account.telegram_chat_id,
                                    &tg_account.telegram_bot_key
                                ).await.unwrap();

                                dest_found = true;
                                continue;
                            }
                        }

                        if !dest_found {
                            // destination address not found on this server, 
                            // pick the first account as default
                            let tg_account = &self.accounts[0];

                            // send telegram message
                            send_to_telegram(
                                &parsed,
                                &tg_account.telegram_chat_id,
                                &tg_account.telegram_bot_key
                            ).await.unwrap();
                        }

                        Ok(())
                    },
                    _ => {
                        // mail is missing 'To' header, 
                        // pick the first account as default
                        let tg_account = &self.accounts[0];

                        // send telegram message
                        send_to_telegram(
                            &parsed,
                            &tg_account.telegram_chat_id,
                            &tg_account.telegram_bot_key
                        ).await.unwrap();

                        Ok(()) // account not found on this server
                    }
                };
                message
            }).unwrap();

        OK
    }
}

async fn send_to_telegram(
    mail: &ParsedMail<'_>,
    chat_id: &str,
    bot_key: &str
) -> Result<(), Box<dyn std::error::Error>>  {

    // Telegram sendMessage api call url
    let telegram_api_url = format!(
        "https://api.telegram.org/bot{}/sendMessage", 
        &bot_key.to_string()
    );

    // Telegram html message
    let body = nanohtml2text::html2text(&mut mail.get_body().unwrap().to_string());
    let telegram_message = format!(
        "from: {}\n<b>{}</b>\n{}", 
        &mail.get_headers().get_first_value("From").unwrap().to_string(),
        &mail.get_headers().get_first_value("Subject").unwrap().to_string(),
        &body.to_string()
    );

    // prepare json post data
    let post_data = json!({
        "chat_id"    : chat_id,
        "text"       : telegram_message,
        "parse_mode" : "html"
    });

    // do post request
    let client = reqwest::Client::new();
    client.post(telegram_api_url)
        .header("Content-type", "application/json")
        .json(&post_data)
        .send()
        .await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    // parse cli arguments
    let cli = Cli::parse();

    // read configuration file
    let configuration = {
        let configuration = std::fs::read_to_string(&cli.config)?;
        serde_json::from_str::<ConfigFile>(&configuration).unwrap()
    };

    println!("[*] tginbox v{}", env!("CARGO_PKG_VERSION"));
   
    let mut handles = vec![];

    for smtpserver in configuration.smtpservers {
        // set up smtp server
        let handler = MyHandler::new(configuration.accounts.clone());
        let mut server = Server::new(handler);

        handles.push(tokio::spawn(async move {
            let ssl_config = {
                if smtpserver.starttls {
                    SslConfig::Trusted {
                        cert_path : smtpserver.cert_path,
                        key_path : smtpserver.key_path,
                        chain_path : smtpserver.ca_path
                    }
                } else {
                    SslConfig::None
                }
            };

            let is_ssl_enabled = {
                if smtpserver.starttls {"yes"} 
                else {"no"}
            };
            println!(
                "[+] starting {} on {}:{} ssl: {}", 
                &smtpserver.hostname, 
                &smtpserver.address, 
                &smtpserver.port,
                is_ssl_enabled
            );

            server.with_name(smtpserver.hostname)
                .with_ssl(ssl_config).unwrap()
                .with_addr(format!(
                    "{}:{}", 
                    smtpserver.address, 
                    smtpserver.port
                )).unwrap();
  
            // start server
            server.serve().unwrap();
        }));
    }
    futures::future::join_all(handles).await;
    Ok(())
}
