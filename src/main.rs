use std::net::IpAddr;
use serde_json::json;
use serde_derive::{Deserialize, Serialize};
use mailin_embedded::{Server, SslConfig, Handler, Response};
use mailin_embedded::response::OK;
use mailparse::MailHeaderMap;
use clap::Parser;

#[derive(Deserialize, Serialize, Clone, Debug)]
struct Message {
    from: String,
    to: String,
    subject: String,
    body: String
}

impl Message {
    pub fn new(mime: &String) -> Message {
        // Get a message from a mime
        // parse mail, return a ParsedMail
        let mail = match mailparse::parse_mail(mime.as_ref()) {
            Ok(p) => p,
            Err(_e) => return Message {
                from: "".to_string(),
                to: "".to_string(),
                subject: "".to_string(),
                body: "".to_string()
            } // error parsing email
        };
        
        Message { 
            from: mail.get_headers().get_first_value("From")
                .unwrap_or_default()
                .to_string(),
            to: mail.get_headers().get_first_value("To")
                .unwrap_or_default()
                .to_string(),
            subject: mail.get_headers().get_first_value("Subject")
                .unwrap_or_default()
                .to_string(),
            body: nanohtml2text::html2text(
                &mut mail.get_body()
                    .unwrap_or_default()
                    .to_string())
        }
    }
}

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
    rt: tokio::runtime::Handle
}

impl MyHandler {
    pub fn new(
        accounts: Vec<Account>, 
        runtime: tokio::runtime::Handle
    ) -> MyHandler {
        MyHandler { 
            mime: vec![],
            accounts: accounts,
            rt: runtime
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
        let msg = Message::new(&mime);
        let accounts = self.accounts.clone();

        self.rt.spawn(async move {
            let destination = find_account(&accounts, &msg.to)
                .await
                .unwrap();
                
            // send telegram message
            send_to_telegram(
                &msg,
                &destination.telegram_chat_id,
                &destination.telegram_bot_key
            )
                .await
                .unwrap();

            ()
        });

        OK
    }
}

async fn send_to_telegram(
    message: &Message,
    chat_id: &str,
    bot_key: &str
) -> Result<(), Box<dyn std::error::Error>>  {

    // Telegram sendMessage api call url
    let telegram_api_url = format!(
        "https://api.telegram.org/bot{}/sendMessage", 
        &bot_key.to_string()
    );

    // Telegram html message
    let telegram_message = format!(
        "\u{1F4E8} {}\n<b>{}</b>\n{}", 
        &message.from,
        &message.subject,
        &message.body
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

async fn find_account<'a>(
        accounts: &'a Vec<Account>, 
        address: &'a String
) -> Option<&'a Account> {
    // Try to find the account with a matching address
    let account = accounts.iter().find(|a| &a.address == address);

    // If we found an account, return it
    if let Some(account) = account {
        return Some(account);
    }

    // If no matching account was found, return the first account in the vector
    accounts.first()
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
        let rt = tokio::runtime::Handle::current();
        let handler = MyHandler::new(configuration.accounts.clone(), rt.clone());
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
