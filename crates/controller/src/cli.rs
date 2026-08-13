use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Validate {
        manifest: PathBuf,
        json: bool,
    },
    Run {
        manifest: PathBuf,
        output: PathBuf,
        adapters: Vec<String>,
        credentials: Vec<String>,
        base_port: u16,
        multicast_port: u16,
        no_color: bool,
    },
    Replay {
        log: PathBuf,
        json: bool,
        no_color: bool,
        width: usize,
    },
    Inspect {
        log: PathBuf,
        sequence: u64,
        json: bool,
    },
    Report {
        input: PathBuf,
        output: PathBuf,
    },
    Doctor {
        json: bool,
    },
    Help,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("missing command")]
    MissingCommand,
    #[error("missing value for {0}")]
    MissingValue(String),
    #[error("unexpected argument {0}")]
    Unexpected(String),
    #[error("invalid value for {flag}: {value}")]
    InvalidValue { flag: String, value: String },
}

impl Cli {
    /// Parse command-line arguments without reading process-global state.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, malformed, or unexpected arguments.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, ParseError> {
        let mut args: Vec<String> = args.into_iter().collect();
        if args.is_empty() {
            return Err(ParseError::MissingCommand);
        }
        let command = args.remove(0);
        let command = match command.as_str() {
            "help" | "--help" | "-h" => Command::Help,
            "validate" => parse_validate(args)?,
            "run" => parse_run(args)?,
            "replay" => parse_replay(args)?,
            "inspect" => parse_inspect(args)?,
            "report" => parse_report(args)?,
            "doctor" => parse_doctor(args)?,
            _ => return Err(ParseError::Unexpected(command)),
        };
        Ok(Self { command })
    }
}

fn parse_report(mut args: Vec<String>) -> Result<Command, ParseError> {
    let input = PathBuf::from(take_positional(&mut args, "MATCH_OR_MATCHES_DIR")?);
    let output = if args.iter().any(|arg| arg == "--output") {
        PathBuf::from(take_flag_value(&mut args, "--output")?)
    } else {
        PathBuf::from("site")
    };
    reject_remaining(args)?;
    Ok(Command::Report { input, output })
}

fn take_positional(args: &mut Vec<String>, name: &str) -> Result<String, ParseError> {
    if args.is_empty() || args[0].starts_with('-') {
        Err(ParseError::MissingValue(name.to_owned()))
    } else {
        Ok(args.remove(0))
    }
}

fn take_flag_value(args: &mut Vec<String>, flag: &str) -> Result<String, ParseError> {
    let index = args
        .iter()
        .position(|arg| arg == flag)
        .ok_or_else(|| ParseError::MissingValue(flag.to_owned()))?;
    if index + 1 >= args.len() {
        return Err(ParseError::MissingValue(flag.to_owned()));
    }
    args.remove(index);
    Ok(args.remove(index))
}

fn take_bool(args: &mut Vec<String>, flag: &str) -> bool {
    if let Some(index) = args.iter().position(|arg| arg == flag) {
        args.remove(index);
        true
    } else {
        false
    }
}

fn reject_remaining(args: Vec<String>) -> Result<(), ParseError> {
    if let Some(value) = args.into_iter().next() {
        Err(ParseError::Unexpected(value))
    } else {
        Ok(())
    }
}

fn parse_validate(mut args: Vec<String>) -> Result<Command, ParseError> {
    let manifest = PathBuf::from(take_positional(&mut args, "MANIFEST")?);
    let json = take_bool(&mut args, "--json");
    reject_remaining(args)?;
    Ok(Command::Validate { manifest, json })
}

fn parse_run(mut args: Vec<String>) -> Result<Command, ParseError> {
    let manifest = PathBuf::from(take_positional(&mut args, "MANIFEST")?);
    let output = if args.iter().any(|arg| arg == "--output") {
        PathBuf::from(take_flag_value(&mut args, "--output")?)
    } else {
        PathBuf::from("matches/latest")
    };
    let base_port = numeric_flag(&mut args, "--base-port", 26000_u16)?;
    let multicast_port = numeric_flag(&mut args, "--multicast-port", 23977_u16)?;
    let no_color = take_bool(&mut args, "--no-color");
    let adapters = repeated_flag(&mut args, "--adapter")?;
    let credentials = repeated_flag(&mut args, "--credential")?;
    reject_remaining(args)?;
    Ok(Command::Run {
        manifest,
        output,
        adapters,
        credentials,
        base_port,
        multicast_port,
        no_color,
    })
}

fn parse_replay(mut args: Vec<String>) -> Result<Command, ParseError> {
    let log = PathBuf::from(take_positional(&mut args, "EVENT_LOG")?);
    let json = take_bool(&mut args, "--json");
    let no_color = take_bool(&mut args, "--no-color");
    let width = numeric_flag(&mut args, "--width", 100_usize)?;
    reject_remaining(args)?;
    Ok(Command::Replay {
        log,
        json,
        no_color,
        width,
    })
}

fn parse_inspect(mut args: Vec<String>) -> Result<Command, ParseError> {
    let log = PathBuf::from(take_positional(&mut args, "EVENT_LOG")?);
    let raw = take_positional(&mut args, "SEQUENCE")?;
    let sequence = raw.parse().map_err(|_| ParseError::InvalidValue {
        flag: "SEQUENCE".into(),
        value: raw,
    })?;
    let json = take_bool(&mut args, "--json");
    reject_remaining(args)?;
    Ok(Command::Inspect {
        log,
        sequence,
        json,
    })
}

fn parse_doctor(mut args: Vec<String>) -> Result<Command, ParseError> {
    let json = take_bool(&mut args, "--json");
    reject_remaining(args)?;
    Ok(Command::Doctor { json })
}

fn repeated_flag(args: &mut Vec<String>, flag: &str) -> Result<Vec<String>, ParseError> {
    let mut values = Vec::new();
    while args.iter().any(|arg| arg == flag) {
        values.push(take_flag_value(args, flag)?);
    }
    Ok(values)
}

fn numeric_flag<T>(args: &mut Vec<String>, flag: &str, default: T) -> Result<T, ParseError>
where
    T: std::str::FromStr,
{
    if !args.iter().any(|arg| arg == flag) {
        return Ok(default);
    }
    let value = take_flag_value(args, flag)?;
    value.parse().map_err(|_| ParseError::InvalidValue {
        flag: flag.to_owned(),
        value,
    })
}
