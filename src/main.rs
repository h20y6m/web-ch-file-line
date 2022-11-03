use std::env;
use std::process;

use web_ch_file_line::Config;

fn main() {
    let args: Vec<String> = env::args().collect();

    let config = Config::new(&args).unwrap_or_else(|err| {
        println!("Problem parsing arguments: {}", err);
        process::exit(1);
    });

    if let Err(e) = web_ch_file_line::run(config) {
        println!("Application error: {}", e);

        process::exit(1);
    }
}
