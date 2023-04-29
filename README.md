# tginbox

tginbox is a small Rust-based server that listens for incoming SMTP email messages and forwards them to a Telegram chat.

## Usage

Multiple accounts can be defined in the configuration file, as shown in the example [config.json](config.json) included in this repository. 
For each account, you can specify the Telegram bot and chat ID to which incoming email messages should be forwarded.

To start tginbox with your configured accounts, run the following command:

```
$ tginbox /path/to/config.json
```
