use clap::Parser;
use mailin_embedded::{Server, SslConfig};

use tginbox::*;

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

    #[cfg(feature = "unixdaemon")]
    if cli.daemonize {
        // Fork and detach from terminal
        match nix::unistd::daemon(false, false) {
            Ok(_) => println!("[+] running as a daemon."),
            Err(e) => {
                eprintln!("[-] failed to daemonize process: {}", e);
                std::process::exit(1);
            }
        }
    }

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
