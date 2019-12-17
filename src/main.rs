#[macro_use]
extern crate clap;

use clap::{AppSettings, ArgMatches};
use serde::Deserialize;
use std::error::Error;
use std::fs;
use trello::{Board, Client};

#[derive(Deserialize, Debug)]
struct Config {
    host: String,
    token: String,
    key: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let matches = clap_app!(myapp =>
        (version: "1.0")
        (author: "Michael Aquilina")
        (about: "Trello CLI interface")
        (@subcommand board =>
            (about: "Commands related to Trello boards")
            (@arg name: -n --name +takes_value "specify board name")
        )
    )
    .setting(AppSettings::SubcommandRequiredElseHelp)
    .get_matches();

    let mut config_path = dirs::config_dir().expect("Unable to determine config directory");
    config_path.push("tro/config.toml");

    let contents = fs::read_to_string(config_path.to_str().unwrap())?;

    let config: Config = toml::from_str(&contents)?;

    let client = Client::new(&config.host, &config.token, &config.key);

    if let Some(matches) = matches.subcommand_matches("board") {
        board_subcommand(&client, &matches)?;
    }
    return Ok(());
}

fn board_subcommand(client: &Client, matches: &ArgMatches) -> Result<(), Box<dyn Error>> {
    if let Some(board_name) = matches.value_of("name") {
        if let Some(board) = get_board_by_name(&client, board_name)? {
            render_board(&board);
        } else {
            println!("Could not find target board: '{}'", board_name);
        }
    } else {
        println!("You must specify a filter");
    }
    return Ok(());
}

// TODO Consider making this a trait for each Trello struct
fn render_board(board: &Board) {
    println!("{}", board.name);
}

fn get_board_by_name(client: &Client, name: &str) -> Result<Option<Board>, Box<dyn Error>> {
    let boards = Board::get_all(&client)?;

    for board in boards {
        if board.name == name {
            return Ok(Some(board));
        }
    }
    return Ok(None);
}
