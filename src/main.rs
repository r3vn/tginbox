use clap::Parser;
use mailin_embedded::{Server, SslConfig};
use log::LevelFilter;

use tginbox::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    // Parse cli arguments
    let cli = Cli::parse();

    // Read configuration file
    let configuration = {
        let configuration = std::fs::read_to_string(&cli.config)?;
        serde_json::from_str::<ConfigFile>(&configuration).unwrap()
    };

    let mut handles = vec![];

    // Init logger
    env_logger::Builder::new()
        .filter_level(match cli.verbose {
            0 => LevelFilter::Off,
            1 => LevelFilter::Error,
            2 => LevelFilter::Warn,
            3 => LevelFilter::Info,
            4 => LevelFilter::Debug,
            5.. => LevelFilter::Trace,
        })
        .init();

    println!("[*] tginbox v{}", env!("CARGO_PKG_VERSION"));
   
    #[cfg(feature = "unixdaemon")]
    if cli.daemonize {
        // Fork and detach from terminal
        match nix::unistd::daemon(false, false) {
            Ok(_) => log::info!("[+] running as a daemon."),
            Err(e) => {
                log::error!("[-] failed to daemonize process: {}", e);
                std::process::exit(1);
            }
        }
    }

    for smtpserver in configuration.smtpservers {
        // Set up smtp server
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
            log::info!(
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
  
            // Start server
            server.serve().unwrap();
        }));
    }
    futures::future::join_all(handles).await;
    Ok(())
}
