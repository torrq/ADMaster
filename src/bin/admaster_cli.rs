use admaster::cli::{execute_headless, parse_cli_args_headless, CliAction};
use admaster::config::AppConfig;

fn main() {
    let cfg = AppConfig::load();
    let args: Vec<String> = std::env::args().skip(1).collect();

    match parse_cli_args_headless(&args, &cfg) {
        CliAction::LaunchGui(_) => {
            std::process::exit(0);
        }
        CliAction::PrintAndExit(code, msg) => {
            if code == 0 {
                println!("{}", msg);
            } else {
                eprintln!("{}", msg);
            }
            std::process::exit(code);
        }
        CliAction::RunHeadless {
            target,
            config,
            no_render,
            json_output,
            show_progress,
        } => {
            if let Err(err) = execute_headless(target, config, no_render, json_output, show_progress)
            {
                eprintln!("Error: {}", err);
                std::process::exit(1);
            }
        }
    }
}
