#[macro_use]
extern crate clap;
#[macro_use]
extern crate simple_error;
#[macro_use]
extern crate log;
extern crate simplelog;

#[cfg(test)]
mod test_main;

use clap::ArgMatches;
use regex::RegexBuilder;
use serde::Deserialize;
use simplelog::{CombinedLogger, Config, LevelFilter, TermLogger, TerminalMode};
use std::error::Error;
use std::io::{stdin, Read, Write};
use std::process::Command;
use std::{env, fs};
use tempfile::Builder;
use trello::{Board, Card, Client, List, TrelloObject};

const CARD_DESCRIPTION_PLACEHOLDER: &str = "<Enter Description Here>";
const CARD_NAME_PLACEHOLDER: &str = "<Enter Card Name Here>";

#[derive(Deserialize, Debug)]
struct TrelloConfig {
    host: String,
    token: String,
    key: String,
}

// TODO: Tests for all the subcommands
// TODO: Better Trello API interface
// TODO: Wildcards for easier patterns
// TODO: Filter by Label
// e.g. tro close TODO - "some card"
// closes the card "some card" searching all lists in the TODO board

fn main() -> Result<(), Box<dyn Error>> {
    let matches = clap_app!(TrelloCLI =>
        (version: env!("CARGO_PKG_VERSION"))
        (author: env!("CARGO_PKG_AUTHORS"))
        (about: env!("CARGO_PKG_DESCRIPTION"))
        (@arg log_level: -l --("log-level") +takes_value default_value[ERROR] "Specify the log level")
        (@subcommand show =>
            (about: "Show object contents")
            (@arg board_name: !required "Board Name to retrieve")
            (@arg list_name: !required "List Name to retrieve")
            (@arg card_name: !required "Card Name to retrieve")
            (@arg ignore_case: -i --("ignore-case") "Ignore case when searching")
            (@arg new: -n --new requires("list_name") conflicts_with("card_name") "Create new Card")
        )
        (@subcommand close =>
            (about: "Close objects")
            (@arg board_name: +required "Board Name to retrieve")
            (@arg list_name: !required "List Name to retrieve")
            (@arg card_name: !required "Card Name to retrieve")
            (@arg ignore_case: -i --("ignore-case") "Ignore case when searching")
        )
        (@subcommand create =>
            (about: "Create objects")
            (@arg board_name: !required "Board Name to retrieve")
            (@arg list_name: !required "List Name to retrieve")
            (@arg ignore_case: -i --("ignore-case") "Ignore case when searching")
        )
    )
    .get_matches();

    let log_level = match matches
        .value_of("log_level")
        .unwrap()
        .to_uppercase()
        .as_str()
    {
        "TRACE" => LevelFilter::Trace,
        "DEBUG" => LevelFilter::Debug,
        "INFO" => LevelFilter::Info,
        "ERROR" => LevelFilter::Error,
        unknown => panic!("Unknown log level '{}'", unknown),
    };

    CombinedLogger::init(vec![TermLogger::new(
        log_level,
        Config::default(),
        TerminalMode::Mixed,
    )
    .unwrap()])
    .unwrap();

    let config = load_config()?;
    let client = Client::new(&config.host, &config.token, &config.key);

    debug!("Loaded configuration: {:?}", config);

    if let Some(matches) = matches.subcommand_matches("show") {
        show_subcommand(&client, &matches)?;
    } else if let Some(matches) = matches.subcommand_matches("close") {
        close_subcommand(&client, &matches)?;
    } else if let Some(matches) = matches.subcommand_matches("create") {
        create_subcommand(&client, &matches)?;
    } else {
        println!("{}", matches.usage());
    }
    Ok(())
}

fn load_config() -> Result<TrelloConfig, Box<dyn Error>> {
    let mut config_path = dirs::config_dir().expect("Unable to determine config directory");
    config_path.push("tro/config.toml");

    debug!("Loading configuration from {:?}", config_path);
    let contents = fs::read_to_string(config_path.to_str().unwrap())?;

    Ok(toml::from_str(&contents)?)
}

#[derive(Debug, PartialEq)]
struct TrelloResult {
    board: Option<Board>,
    list: Option<List>,
    card: Option<Card>,
}

fn get_trello_object(
    client: &Client,
    matches: &ArgMatches,
) -> Result<TrelloResult, Box<dyn Error>> {
    let board_name = match matches.value_of("board_name") {
        Some(bn) => bn,
        None => {
            return Ok(TrelloResult {
                board: None,
                list: None,
                card: None,
            })
        }
    };
    let boards = Board::get_all(&client)?;
    let ignore_case = matches.is_present("ignore_case");
    let board = get_object_by_name(boards, &board_name, ignore_case)?;

    if let Some(list_name) = matches.value_of("list_name") {
        let lists = Board::get_all_lists(client, &board.id, true)?;
        let list = get_object_by_name(lists, &list_name, ignore_case)?;
        if let Some(card_name) = matches.value_of("card_name") {
            let cards = List::get_all_cards(client, &list.id)?;

            let card = get_object_by_name(cards, &card_name, ignore_case)?;
            return Ok(TrelloResult {
                board: Some(board),
                list: Some(list),
                card: Some(card),
            });
        } else {
            return Ok(TrelloResult {
                board: Some(board),
                list: Some(list),
                card: None,
            });
        }
    } else {
        return Ok(TrelloResult {
            board: Some(board),
            list: None,
            card: None,
        });
    }
}

/// Opens the users chosen editor (specified by the $EDITOR environment variable)
/// to edit a specified card.
///
/// Once the editor is closed, a new card is populated and returned based on the
/// contents of what was written by the editor.
fn edit_card(card: &Card) -> Result<Card, Box<dyn Error>> {
    let mut file = Builder::new().suffix(".md").tempfile()?;
    let editor_env = env::var("EDITOR").unwrap_or(String::from("vi"));

    debug!("Using editor: {}", editor_env);
    debug!("Editing card: {:?}", card);

    writeln!(file, "{}", card.render())?;

    let editor = Command::new(editor_env).arg(file.path()).status()?;

    debug!("editor exited with {:?}", editor);

    let mut buf = String::new();
    file.reopen()?.read_to_string(&mut buf)?;

    // Trim end because a lot of editors will auto add new lines at the end of the file
    let mut new_card = Card::parse(buf.trim_end())?;
    new_card.id = String::from(&card.id);
    new_card.labels = card.labels.clone();

    debug!("New card: {:?}", new_card);

    Ok(new_card)
}

fn show_subcommand(client: &Client, matches: &ArgMatches) -> Result<(), Box<dyn Error>> {
    debug!("Running show subcommand with {:?}", matches);

    let result = get_trello_object(client, matches)?;

    trace!("result: {:?}", result);

    if matches.is_present("new") {
        // we can safely unwrap the list due to the way we've setup clap
        let list_id = &result.list.unwrap().id;
        let card = Card::new("", CARD_NAME_PLACEHOLDER, CARD_DESCRIPTION_PLACEHOLDER);

        let mut card = edit_card(&card)?;

        // if nothing is edited by the user, remove it
        if card.desc == CARD_DESCRIPTION_PLACEHOLDER {
            card.desc = String::from("");
        }

        if card.name != CARD_NAME_PLACEHOLDER {
            let result = Card::create(client, list_id, &card)?;
            eprintln!("Created new card with id {}", result.id);
        } else {
            eprintln!("Card name not entered");
        }
    } else {
        if let Some(card) = result.card {
            let new_card = edit_card(&card)?;
            if new_card != card {
                eprintln!("Changes detected - uploading card contents");
                Card::update(client, &new_card)?;
            }
        } else if let Some(list) = result.list {
            println!("{}", list.render());
        } else if let Some(mut board) = result.board {
            board.retrieve_nested(client)?;
            println!("{}", board.render());
        } else {
            let boards = Board::get_all(client)?;
            for b in boards {
                println!("* {}", b.name);
            }
        }
    }

    Ok(())
}

fn close_subcommand(client: &Client, matches: &ArgMatches) -> Result<(), Box<dyn Error>> {
    debug!("Running close subcommand with {:?}", matches);

    let result = get_trello_object(client, matches)?;

    trace!("result: {:?}", result);

    if let Some(mut card) = result.card {
        card.closed = true;
        Card::update(client, &card)?;
        eprintln!("Closed card '{}'", &card.name);
    } else if let Some(mut list) = result.list {
        list.closed = true;
        List::update(client, &list)?;
        eprintln!("Closed list '{}'", &list.name);
    } else if let Some(mut board) = result.board {
        board.closed = true;
        Board::update(client, &board)?;
        eprintln!("Closed board '{}'", &board.name);
    }

    Ok(())
}

fn create_subcommand(client: &Client, matches: &ArgMatches) -> Result<(), Box<dyn Error>> {
    debug!("Running create subcommand with {:?}", matches);

    let result = get_trello_object(client, matches)?;

    trace!("result: {:?}", result);

    let mut input = String::new();

    if let Some(list) = result.list {
        eprint!("Card name: ");
        stdin().read_line(&mut input)?;

        Card::create(client, &list.id, &Card::new("", &input.trim_end(), ""))?;
    } else if let Some(board) = result.board {
        eprint!("List name: ");
        stdin().read_line(&mut input)?;

        List::create(client, &board.id, &input.trim_end())?;
    } else {
        eprint!("Board name: ");
        stdin().read_line(&mut input)?;

        Board::create(client, &input.trim_end())?;
    }

    Ok(())
}

fn get_object_by_name<T: TrelloObject>(
    objects: Vec<T>,
    name: &str,
    ignore_case: bool,
) -> Result<T, simple_error::SimpleError> {
    let re = RegexBuilder::new(name)
        .case_insensitive(ignore_case)
        .build()
        .expect("Invalid Regex");

    let mut objects = objects
        .into_iter()
        .filter(|o| re.is_match(&o.get_name()))
        .collect::<Vec<T>>();

    if objects.len() == 1 {
        Ok(objects.pop().unwrap())
    } else if objects.len() > 1 {
        bail!(
            "More than one object found. Specify a more precise filter than '{}'",
            name
        );
    } else {
        bail!(
            "Object not found. Specify a more precise filter than '{}'",
            name
        );
    }
}
