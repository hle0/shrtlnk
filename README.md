# shrtlnk

## What is shrtlnk?

shrtlnk is an open source link shortener. Or longener, if you really want.

It doesn't require another web server, or even a PHP or Python interpreter. 
It compiles down to a single binary, and just needs a single configuration file to run. 

## Why shrtlnk?

There's no need to run a full web server for what is essentially a table of redirects. 
At the same time, it's really awkward to add new entries to this table. 

I decided I wanted to make a link shortener, for my own use - but I didn't want to run a full web server for it. 
I wanted to place emphasis on security, but also ease of use. Hence shrtlnk.

shrtlnk has been specifically designed with this use case in mind:

- Single configuration file controls everything. shrtlnk does not use an external database, and does not write to the filesystem.
- On Unix, the configuration file can be reloaded with `SIGHUP` without stopping the server. In the case of an invalid configuration, it will revert to the old configuration instead of crashing.
- shrtlnk uses Rust, which means it builds easily (`cargo build`), compiles to a native binary, and has better memory safety features than a language like C.

## Get started

Clone this repository and `cd` into it. Make sure you have `rust` installed and updated. Then, run these commands:

```sh
cd shrtlnk
cargo build
cp config.toml.example config.toml
cargo run
# shrtlnk is now running!
```