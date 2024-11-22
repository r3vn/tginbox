use std::{io, thread};
use std::net::IpAddr;
use serde_json::json;
use serde_derive::{Deserialize, Serialize};
use mailin_embedded::{Handler, Response};
use mailin_embedded::response::OK;
use mail_parser::MessageParser;
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
        let p = MessageParser::default().parse(mime.as_str()).unwrap(); 
            
        let body = {
            let mut full_text_body = String::new();
            let mut index = 0;

            // Iterate through all parts
            while let Some(part) = p.body_html(index) {
                full_text_body.push_str(&part);
                full_text_body.push('\n'); // Separate parts if needed
                index += 1;
            }

            if full_text_body.len() > 4086 {
                full_text_body
                    .char_indices()
                    .take_while(|&(idx, _)| idx < 4086)
                    .last()
                    .map(|(idx, _)| idx)
                    .unwrap_or(0);
            }

            nanohtml2text::html2text(&nanohtml2text::html2text(&full_text_body)) // FIXME?? lolol
        };

        log::trace!("Message parsed successfully, body: {}", body);

        Message {
            from: p.from()
                .unwrap()
                .first()
                .unwrap()
                .address()
                .map(|s| s.to_string())
                .unwrap_or_default(),

            to: p.to()
                .unwrap()
                .first()
                .unwrap()
                .address()
                .map(|s| s.to_string())
                .unwrap_or_default(),

            subject: p.subject()
                .unwrap_or_default()
                .to_string(),
            body
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
    // Sets a custom config file
    #[arg(value_name = "FILE", required = true )]
    pub config: String,

    // Optional feature, fork process
    #[cfg(feature = "unixdaemon")]
    #[arg(short, long)]
    pub daemonize: bool,

    // Set verbosity level
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Clone, Debug)]
pub struct MyHandler {
    mime: Vec<String>,
    accounts: Vec<Account>,
}

impl MyHandler {
    pub fn new(
        accounts: Vec<Account>, 
    ) -> MyHandler {
        MyHandler { 
            mime: vec![],
            accounts,
        }
    }
}

impl Handler for MyHandler {
    fn helo(&mut self, _ip: IpAddr, _domain: &str) -> Response {
        log::info!("Received HELO from IP: {}, domain: {}", _ip, _domain);
        OK
    }    

    fn mail(
        &mut self, 
        _ip: IpAddr, 
        _domain: &str, 
        _from: &str
    ) -> Response {
        // Log incoming mail details
        log::info!("Received mail: IP={}, domain={}, from={}", _ip, _domain, _from);

        // hack add from header
        let from = "From: ".to_owned() + _from + "\n";
        self.mime.push(from);
        OK
    }

    fn data(&mut self, buf: &[u8]) -> io::Result<()> {
        // Log the raw data received
        match String::from_utf8(Vec::from(buf)) {
            Ok(data) => {
                log::info!("Received data chunk: {}", data);
                self.mime.push(data);
            }
            Err(err) => {
                log::error!("Failed to decode data chunk: {}", err);
                return Err(io::Error::new(io::ErrorKind::InvalidData, err));
            }
        }
        Ok(())
    }

    fn data_end(&mut self) -> Response {
        // Retrieve mail
        let mime = self.mime.join("");
        log::info!("End of data. Full MIME: {}", mime);

        // Clone accounts for thread safety
        let accounts = self.accounts.clone();

        // Spawn a thread for processing
        thread::spawn(move || {
            // Get a Message
            let msg = Message::new(&mime);

            // Get destination account
            let destination = find_account(&accounts, &msg.to);

            // Send telegram message
            if let Err(e) = send_to_telegram(
                &msg,
                &destination.telegram_chat_id,
                &destination.telegram_bot_key,
            ) {
                log::error!("Failed to send Telegram message: {}", e);
            }
        });

        OK
    }
}

fn send_to_telegram(
    message: &Message,
    chat_id: &str,
    bot_key: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Telegram sendMessage API call URL
    let telegram_api_url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        bot_key
    );

    // Telegram HTML message
    let telegram_message = format!(
        "\u{1F4E8} {}\n<b>{}</b>\n{}",
        &message.from,
        &message.subject,
        &message.body
    );

    // Prepare JSON post data
    let post_data = json!({
        "chat_id": chat_id,
        "text": telegram_message,
        "parse_mode": "html",
    });

    let _req = ureq::post(&telegram_api_url).send_json(post_data);
    
    Ok(())
}

fn find_account<'a>(
    accounts: &'a [Account],
    address: &'a String,
) -> &'a Account {
    // Try to find the account with a matching address
    accounts
        .iter()
        .find(|a| &a.address == address)
        .or_else(|| accounts.first()).unwrap()
}
