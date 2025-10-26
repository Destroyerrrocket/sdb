#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::complexity)]
#![warn(clippy::correctness)]
#![warn(clippy::nursery)]
#![warn(clippy::perf)]
#![warn(clippy::style)]
#![warn(clippy::suspicious)]

use clap::{Args, Parser};
use tracing::subscriber::set_global_default;

mod command;
mod gui;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(long)]
    log_dir: Option<std::path::PathBuf>,

    #[command(flatten)]
    attachment: Attachment,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct Attachment {
    program: Vec<String>,

    #[arg(short, long)]
    pid: Option<u64>,
}

struct Writer(std::io::BufWriter<std::fs::File>);

impl std::io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let buf_len = buf.len();

        self.0.write_all(buf).map(|_| buf_len)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

fn main() {
    let args = Cli::parse();

    let file_log_info = args
        .log_dir
        .map(|log_dir| {
            std::fs::create_dir_all(&log_dir).unwrap();

            let file: std::fs::File = loop {
                // Create a file with the current timestamp to avoid overwriting previous logs
                let file_name = format!("sdb-{}.log", chrono::Local::now().format("%Y%m%d-%H%M%S"));
                let file = std::fs::File::options()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(log_dir.join(&file_name));
                if let Ok(file) = file {
                    break file;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            };

            tracing_appender::non_blocking(Writer(std::io::BufWriter::new(file)))
        })
        .unzip();
    if let (Some(non_blocking), _) = file_log_info {
        set_global_default(
            tracing_subscriber::fmt()
                .with_thread_names(true)
                .with_writer(non_blocking)
                .with_file(true)
                .with_line_number(true)
                .with_level(true)
                .with_ansi(false)
                .with_max_level(tracing::Level::TRACE)
                .finish(),
        )
    } else {
        set_global_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::INFO)
                .with_ansi(true)
                .finish(),
        )
    }
    .expect("setting default subscriber failed");

    let mut debugger = sdblib::Debugger::new();

    if let Some(pid) = args.attachment.pid {
        debugger.add_proc(pid);
    } else if !args.attachment.program.is_empty() {
        debugger.add_program(
            args.attachment.program.first().unwrap(),
            args.attachment.program[1..].iter(),
        );
    }

    debugger.wait();

    let mut gui = gui::Gui::new(debugger);
    gui.run().unwrap();
}
