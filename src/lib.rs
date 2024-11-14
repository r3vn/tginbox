use std::net::IpAddr;
use serde_json::json;
use serde_derive::{Deserialize, Serialize};
use mailin_embedded::{Handler, Response};
use mailin_embedded::response::OK;
use mailparse::MailHeaderMap;
use clap::Parser;

#[derive(Clone, Debug)]
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
                .unwrap_or_default(),
            to: mail.get_headers().get_first_value("To")
                .unwrap_or_default(),
            subject: mail.get_headers().get_first_value("Subject")
                .unwrap_or_default(),
            body: nanohtml2text::html2text(
                &mail.get_body()
                    .unwrap_or_default())
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Account {
    address: String,
    telegram_bot_key: String,
    telegram_chat_id: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct SmtpServer {
    pub enabled: bool,
    pub hostname: String,
    pub address: String,
    pub port: i64,
    pub starttls: bool,
    pub cert_path: String,
    pub key_path: String,
    pub ca_path: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ConfigFile {
    pub smtpservers: Vec<SmtpServer>,
    pub accounts: Vec<Account>,
}

#[derive(Parser)]
pub struct Cli {
    /// Sets a custom config file
    #[arg(value_name = "FILE", required = true )]
    pub config: String,

    #[cfg(feature = "unixdaemon")]
    #[arg(short, long)]
    pub daemonize: bool,
}

#[derive(Clone, Debug)]
pub struct MyHandler {
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
            accounts,
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
        self.mime.push(from);
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
        let accounts = self.accounts.clone();

        self.rt.spawn(async move {
            // get a Message
            let msg = Message::new(&mime);

            // get destination account
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
        accounts: &'a [Account], 
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
