use std::process;
use topcron::Config;

// TODO typed, voir https://github.com/clap-rs/clap/blob/master/examples/typed-derive.md
use clap::Parser;
#[derive(Parser, Debug)]
#[clap(
    author,
    version,
    about = "A Cron log parser and viewer.\n\nTo see status and duration of cronjobs, please add `-L 15` to EXTRA_OPTS of cron, then restart the daemon.",
    long_about = None
)]
struct Args {
    // TODO gérer les globs
    // voir doc https://github.com/clap-rs/clap/tree/master/examples/derive_ref
    #[clap(
        short,
        long,
        default_value = "/var/log/syslog",
        help = "File(s) to parse"
    )]
    file: String,
    #[clap(
        short = 'F',
        long,
        default_value = "",
        help = "Filter (user, pid, command...) specific cronjobs"
    )]
    filter: String,
    #[clap(
        short,
        long,
        default_value = "",
        help = "Filter (user, pid, command...) specific cronjobs"
    )]
    user_filter: String,
    #[clap(short, long, help = "Show debug messages")]
    verbose: bool,
    #[clap(short = '0', long, help = "Display only successfull cronjobs")]
    ok_filter: bool,
    #[clap(
        short = '1',
        long,
        help = "Display only failed cronjobs (include unknow status)"
    )]
    ko_filter: bool,
    #[clap(short, long, help = "Display dates and duration in timestamp format")]
    timestamp_mode: bool,
    #[clap(short = 'T', long, help = "Don't truncate command line")]
    truncate_mode: bool,
}

fn main() {
    let args = Args::parse();
    let file = args.file;
    let filter = args.filter;
    let verbose = args.verbose;
    let user_filter = args.user_filter;
    let ko_filter = args.ko_filter;
    let ok_filter = args.ok_filter;

    // TODO create fn pour print debug
    if verbose {
        println!("Debug mode on");
        println!("DEBUG: filename : {}", file);
    }

    // préparation du filtre
    // TODO 1 regexp de ouf ? ou plusieurs passage du grep ?
    // et à bouger dans lib.rs
    let query = topcron::create_query(&filter, &user_filter, verbose);

    // création de la config avec des args
    let config = Config::new(&query, &file).unwrap_or_else(|err| {
        eprintln!("Problem creating config: {}", err);
        process::exit(1);
    });

    // appel à grep_file et affichage
    match topcron::grep_file(config, verbose) {
        Ok(res) => {
            topcron::display_jobs(res, ko_filter, ok_filter, verbose);
        }
        Err(e) => {
            eprintln!("Application error: {}", e);
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use trycmd;
    #[test]
    fn cli_tests() {
        trycmd::TestCases::new().case("tests/cmd/*.trycmd");
    }
}
