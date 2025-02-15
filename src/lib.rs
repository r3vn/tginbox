use std::{io, thread, string::String, io::Write};
use std::net::IpAddr;
use serde_json::json;
use serde_derive::{Deserialize, Serialize};
use mailin_embedded::{Handler, Response};
use mailin_embedded::response::OK;
use mail_parser::{MimeHeaders, MessageParser};
use clap::Parser;

#[derive(Clone, Debug)]
struct Notification {
    from: String,
    to: String,
    subject: String,
    body: String,
    attachments: Vec<(String, Vec<u8>)>
}

impl Notification {
    pub fn new(mime: &str) -> Notification {
        // Parse MIME
        let p = MessageParser::default().parse(mime).unwrap();

        // Collect attachments
        let attachments = collect_attachments(&p);
        
        // Fix body
        let body = {
            let mut full_text_body = String::new();
            let mut index = 0;

            // Iterate through all parts
            while let Some(part) = p.body_html(index) {
                full_text_body.push_str(&part);
                full_text_body.push('\n'); // Separate parts if needed
                index += 1;
            }

            // Remove HTML from body
            full_text_body = nanohtml2text::html2text(
                &nanohtml2text::html2text(&full_text_body) // FIXME?? lolol
            ); 

            // Cut body if greater than 4086 (4096 is max allowed)
            if full_text_body.len() > 4086 {
                full_text_body
                    .char_indices()
                    .take_while(|&(idx, _)| idx < 4086)
                    .last()
                    .map(|(idx, _)| idx)
                    .unwrap_or(0);
            }

            full_text_body
        };

        // Debug MIME decoding
        log::trace!("Message parsed successfully, body: {}", body);

        // Return Messaage
        Notification {
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
            body,
            attachments
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
        // Log HELO
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
        OK
    }

    fn data(&mut self, buf: &[u8]) -> io::Result<()> {
        // Log the raw data received
        match String::from_utf8(Vec::from(buf)) {
            Ok(data) => {
                log::debug!("Received data chunk: {}", data);
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
            let msg = Notification::new(&mime);

            // Get destination account
            let destination = find_account(&accounts, &msg.to);

            // Send telegram message
            send_to_telegram(
                &msg,
                &destination.telegram_chat_id,
                &destination.telegram_bot_key,
            )
        });

        OK
    }
}

fn send_to_telegram(
    message: &Notification,
    chat_id: &str,
    bot_key: &str,
) {
    // Telegram sendMessage API call URL
    let telegram_message_url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        bot_key
    );

    // Telegram sendMessage API call URL
    let telegram_document_url = format!(
        "https://api.telegram.org/bot{}/sendDocument",
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

    // Send Message
    match ureq::post(&telegram_message_url).send_json(post_data) {
        Ok(response) => {
            log::debug!("Message sent to telegram successfully");
            log::trace!("Response: {:#?}", response);
        },
        Err(ureq::Error::StatusCode(code)) => {
            log::error!("Failed to send Telegram message, error code: {}", code);
            //log::trace!("Response: {:#?}", response);
        },
        Err(_) => log::error!("Failed to send Telegram message, transport error")
    };

    // Send Attachments
    for (name, content) in &message.attachments {

        // Get Multipart data
        let (body, boundary) = build_multipart(name.to_string(), content.to_vec(), chat_id.to_string());

        // Send the request
        let response = ureq::post(&telegram_document_url)
            .header("Content-Type", &format!("multipart/form-data; boundary={}", boundary))
            .send(&body);

        // Handle response
        match response {
            Ok(res) => {
                log::debug!("Sent file {}: {}", name, res.status());
            }
            Err(err) => {
                log::error!("Failed to send file {}: {}", name, err);
            }
        }
    }
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

fn collect_attachments(message: &mail_parser::Message) -> Vec<(String, Vec<u8>)> {
    // Recursively collect mail attachments into a vector

    let mut attachments = Vec::new();

    for attachment in message.attachments() {
        if !attachment.is_message() {
            let name = attachment.attachment_name().unwrap_or("Untitled").to_string();
            let contents = attachment.contents().to_vec(); // Store contents in memory
            attachments.push((name, contents));
        } else {
            attachments.extend(collect_attachments(attachment.message().unwrap()));
        }
    }
    attachments
}

fn build_multipart(
    name: String, 
    content: Vec<u8>, 
    chat_id: String
) -> (Vec<u8>, String) {
    // Build multipart for file upload
    let boundary = "------------------------boundary";

    // Construct the multipart body
    let mut body = Vec::new();

    // Add the `chat_id` field
    write!(
        body,
        "--{}\r\nContent-Disposition: form-data; name=\"chat_id\"\r\n\r\n{}\r\n",
        boundary, chat_id
    ).unwrap();

    // Add the file (document)
    write!(
        body,
        "--{}\r\nContent-Disposition: form-data; name=\"document\"; filename=\"{}\"\r\n\
         Content-Type: application/octet-stream\r\n\r\n",
        boundary, name
    ).unwrap();
    body.extend(content);
    write!(body, "\r\n--{}--\r\n", boundary).unwrap();

    (body, boundary.to_string())
}
