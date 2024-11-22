use std::thread;
use clap::Parser;
use log::LevelFilter;
use mailin_embedded::{Server, SslConfig};

use tginbox::{Cli, ConfigFile, MyHandler};

fn main() -> Result<(), Box<dyn std::error::Error>> {

    println!("[*] tginbox v{}", env!("CARGO_PKG_VERSION"));

    // Parse CLI arguments
    let cli = Cli::parse();

    // Read configuration file
    let configuration = {
        let config_content = std::fs::read_to_string(&cli.config).unwrap();
        serde_json::from_str::<ConfigFile>(&config_content)
    }.unwrap();

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

    #[cfg(feature = "unixdaemon")]
    if cli.daemonize {
        // Fork and detach from terminal
        match nix::unistd::daemon(false, false) {
            Ok(_) => log::info!("[+] Running as a daemon."),
            Err(e) => {
                log::error!("[-] Failed to daemonize process: {}", e);
                std::process::exit(1);
            }
        }
    }

    let mut handles = vec![];

    for smtpserver in configuration.smtpservers {
        // Clone configuration accounts for each thread
        let accounts = configuration.accounts.clone();

        // Set up smtp server in a separate thread
        let handle = thread::spawn(move || {
            let handler = MyHandler::new(accounts);
            let mut server = Server::new(handler);

            let ssl_config = if smtpserver.starttls {
                SslConfig::Trusted {
                    cert_path: smtpserver.cert_path,
                    key_path: smtpserver.key_path,
                    chain_path: smtpserver.ca_path,
                }
            } else {
                SslConfig::None
            };

            let is_ssl_enabled = if smtpserver.starttls { "yes" } else { "no" };
            log::info!(
                "[+] Starting {} on {}:{} SSL: {}",
                &smtpserver.hostname,
                &smtpserver.address,
                &smtpserver.port,
                is_ssl_enabled
            );

            server
                .with_name(smtpserver.hostname)
                .with_ssl(ssl_config)
                .unwrap()
                .with_addr(format!("{}:{}", smtpserver.address, smtpserver.port))
                .unwrap();

            // Start server
            if let Err(e) = server.serve() {
                log::error!("[-] Error in server: {}", e);
            }
        });

        handles.push(handle);
    }

    // Wait for all threads to finish
    for handle in handles {
        if let Err(e) = handle.join() {
            log::error!("[-] Error in thread: {:?}", e);
        }
    }

    Ok(())
}
