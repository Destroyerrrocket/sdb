use ariadne::{Label, Report, ReportKind, Source};
use chumsky::prelude::*;
use color_eyre::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ErrorKind {
    UnexpectedCommand(String),
}

#[derive(Clone, Debug)]
enum Commands {
    Continue,
    Exit,
    Sequence(Vec<Self>),
    Error(ErrorKind),
}

fn parser<'a>() -> impl Parser<'a, &'a str, Commands, extra::Err<Rich<'a, char>>> {
    let error_command = any()
        .filter(|c: &char| c.is_ascii_alphabetic())
        .repeated()
        .at_least(1)
        .collect::<String>()
        .map(|s: String| ErrorKind::UnexpectedCommand(s));

    let single_command = choice((
        just("continue").padded().to(Commands::Continue),
        just("exit").padded().to(Commands::Exit),
    ))
    .recover_with(via_parser(error_command.map(Commands::Error)));

    single_command
        .separated_by(just(";"))
        .collect::<Vec<_>>()
        .map(Commands::Sequence)
}

fn parse_command(command_str: &str, mut output: &mut dyn std::io::Write) -> Option<Commands> {
    let (command, errs) = parser().parse(command_str.trim()).into_output_errors();

    for e in errs {
        Report::build(ReportKind::Error, ((), e.span().into_range()))
            .with_config(
                ariadne::Config::new()
                    .with_index_type(ariadne::IndexType::Byte)
                    .with_color(false),
            )
            .with_message(e.to_string())
            .with_label(
                Label::new(((), e.span().into_range())).with_message(e.reason().to_string()),
            )
            .finish()
            .write(Source::from(&command_str), &mut output)
            .unwrap();
    }

    command
}

fn run_command_ast(
    command: Commands,
    debugger: &mut sdblib::Debugger,
    mut output: &mut dyn std::io::Write,
) -> Result<bool> {
    match command {
        Commands::Continue => {
            debugger.continue_execution()?;
        }
        Commands::Exit => {
            return Ok(false);
        }
        Commands::Sequence(commands) => {
            for cmd in commands {
                if !run_command_ast(cmd, debugger, &mut output)? {
                    return Ok(false);
                }
            }
        }
        Commands::Error(err) => {
            writeln!(output, "Error: {err:?}")?;
        }
    }

    Ok(true)
}

pub fn run_command(
    command: &str,
    debugger: &mut sdblib::Debugger,
    mut output: &mut dyn std::io::Write,
) -> Result<bool> {
    let Some(command) = parse_command(command, output) else {
        return Ok(true);
    };
    run_command_ast(command, debugger, &mut output)
}
