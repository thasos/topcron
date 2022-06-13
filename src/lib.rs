#[macro_use]
extern crate prettytable;
extern crate textwrap;
//extern crate unicode_width;
use chrono::prelude::*;
use grep::cli::DecompressionReader;
use grep::searcher::sinks::UTF8;
use grep::searcher::Searcher;
use prettytable::{format, Cell, Row, Table};
use regex::Regex;
use std::process;
// grep_pcre2 for look-around regex
use grep_pcre2::RegexMatcher;
use std::collections::BTreeMap;
use std::error::Error;
use std::io::Read;

pub struct Config {
    pub query: String,
    pub filename: String,
}
impl Config {
    pub fn new(query: &str, filename: &str) -> Result<Config, &'static str> {
        let query = String::from(query);
        let filename = String::from(filename);
        Ok(Config { query, filename })
    }
}

pub fn create_query(filter: &str, user_filter: &str, _verbose: bool) -> String {
    // pour des mots avec position inconnue : look-around regex
    // https://stackoverflow.com/questions/4389644/regex-to-match-string-containing-two-names-in-any-order
    // exemple: ^(?=.*\bjack\b)(?=.*\bjames\b).*$
    // TODO attention ça marche pas bien :
    // `-F backupDataViaRsync` est OK, mais pas `-F backup`
    let mut query = String::from("^(?=.*\\bCRON\\b)");
    if user_filter != "" {
        let user_filter = format!("(?=.*\\b{}\\b)", user_filter);
        query.push_str(&user_filter);
    }
    if filter != "" {
        let filter = format!("(?=.*\\b{}\\b)", filter);
        query.push_str(&filter);
    }
    query.push_str(".*$");
    query
}

struct Cronjob {
    pid: i32,
    status: JobStatus,
    user: Option<String>,
    hostname: Option<String>,
    start_date: Option<chrono::NaiveDateTime>,
    end_date: Option<chrono::NaiveDateTime>,
    duration: Option<i64>,
    start_line: Option<String>,
    // si les 2 sont None, ça veut dire que ya ptet pas la conf (-L 15) : faudra afficher un warn
    message: Option<String>,
    end_line: Option<String>,
}
impl Cronjob {
    fn get_user(&self) -> &str {
        match &self.user {
            Some(user) => user.as_str(),
            _ => "",
        }
    }
    fn get_dates(&self) -> String {
        // faut gérer les cas où il manque une des 2 dates
        let mut dates = String::from("");
        match (self.start_date, self.end_date) {
            (Some(start), Some(end)) => dates = format!("{}\n{}", start, end),
            (Some(start), None) => dates = format!("{}", start),
            (None, Some(end)) => dates = format!("{}", end),
            _ => (),
        }
        dates
    }
    fn set_duration(&mut self) {
        match (self.start_date, self.end_date) {
            (Some(start), Some(end)) => {
                let duration = end - start;
                // TODO virer num_seconds et gérer un affichage sympa ?
                self.duration = Some(duration.num_seconds());
            }
            _ => (),
        }
    }
    fn get_duration(&self) -> String {
        match &self.duration {
            Some(duration) => duration.to_string(),
            _ => String::from("unknow"),
        }
    }
    fn get_command(&self) -> String {
        match &self.start_line {
            Some(command) => command.to_owned(),
            _ => String::from(""),
        }
    }
}

enum JobStatus {
    Ok,
    Failed,
    Unknow,
}

fn parse_date(date: String, year: i32) -> Option<chrono::NaiveDateTime> {
    // Mar 23 14:45:01
    let date_with_year = format!("{} {}", year, date);
    let parsed = NaiveDateTime::parse_from_str(date_with_year.as_str(), "%Y %b %d %T");
    match parsed {
        Ok(parsed) => return Some(parsed),
        Err(e) => {
            println!("err {}", e);
            None
        }
    }
}
fn status_filter(ko_filter: bool, ok_filter: bool, verbose: bool) {
    if verbose {
        println!("DEBUG: ko:{:?} ok:{:?}", ko_filter, ok_filter);
    }
}

// TODO retourner la ref mutable du job pour le modifier directement ?
fn create_job_if_needed(cronjobs: &mut BTreeMap<i32, Cronjob>, pid: i32) {
    // si ce pid est déjà en mémoire, on ajoute juste des champs, sinon on le crée
    match cronjobs.get(&pid) {
        Some(_) => (),
        _ => {
            let job = Cronjob {
                pid: pid,
                user: None,
                hostname: None,
                start_date: None,
                end_date: None,
                start_line: None,
                message: None,
                end_line: None,
                status: JobStatus::Unknow,
                duration: None,
            };
            cronjobs.insert(pid, job);
            ()
        }
    }
}

fn create_cronjobs_list(res: &Vec<String>, verbose: bool) -> Option<BTreeMap<i32, Cronjob>> {
    let re_cron_log = match Regex::new(
        r"^(?P<date>.*) (?P<hostname>.*) CRON\[(?P<pid>[0-9]+)\]: \((?P<user>.*)\) (?P<logtype>(CMD|END|error)) (?P<message>.*)",
    ) {
        Ok(re) => re,
        Err(error) => {
            eprintln!("Problem creating regex to parse cron log: {}", error);
            process::exit(1);
        }
    };
    if verbose {
        println!("DEBUG: regex pid: {:?}", re_cron_log);
    }
    let mut cronjobs: BTreeMap<i32, Cronjob> = BTreeMap::new();
    let current_year = Local::now().year();
    for line in res {
        match re_cron_log.captures(&line) {
            None => (),
            Some(matched_line) => {
                // parse des différents champs
                match matched_line.name("pid")?.as_str().parse() {
                    Ok(pid) => {
                        let pid = pid;
                        let user = matched_line.name("user")?.as_str().to_string();
                        let hostname = matched_line.name("hostname")?.as_str().to_string();
                        let date = matched_line.name("date")?.as_str().to_string();
                        let logtype = matched_line.name("logtype")?.as_str().to_string();
                        // TODO virer le [pid] si il est dans le (et why il y est pas tout le temps ??)
                        let message = matched_line.name("message")?.as_str().to_string();

                        // selon le type de log, on va définir le start, end, ou fail
                        create_job_if_needed(&mut cronjobs, pid);
                        match cronjobs.get_mut(&pid) {
                            Some(job) => {
                                match logtype.as_str() {
                                    "CMD" => {
                                        job.start_line = Some(message);
                                        job.user = Some(user);
                                        job.hostname = Some(hostname);
                                        //job.start_date = Some(date);
                                        job.start_date = parse_date(date, current_year);
                                    }
                                    "END" => {
                                        job.end_line = Some(message);
                                        job.user = Some(user);
                                        job.end_date = parse_date(date, current_year);
                                        Cronjob::set_duration(job);
                                        match job.status {
                                            JobStatus::Failed => (),
                                            _ => job.status = JobStatus::Ok,
                                        }
                                    }
                                    "error" => {
                                        job.message = Some(message);
                                        job.end_date = parse_date(date, current_year);
                                        job.status = JobStatus::Failed;
                                    }
                                    // TODO afficher fichier et numero ligne
                                    _ => eprintln!("Some line are not CRON log."),
                                }
                            }
                            None => (),
                        }
                    }
                    Err(_) => {
                        println!("Warnig, unable to parse following line : {}", line);
                    }
                };
            }
        }
    }
    // TODO trier la map cronjobs par date
    return Some(cronjobs);
}

pub fn display_jobs(res: Vec<String>, ko_filter: bool, ok_filter: bool, verbose: bool) {
    // pour avoir la taille du terminal
    // voir https://github.com/phsym/prettytable-rs/issues/47

    // maintenant on affiche vraiment
    match create_cronjobs_list(&res, verbose) {
        None => (),
        Some(cronjobs) => {
            let mut table = Table::new();
            table.set_format(*format::consts::FORMAT_BOX_CHARS);
            table.add_row(row![
                b->"PID", b->"USER", b->"STATUS", b->"DATES", b->"DURATION", b->"COMMAND"
            ]);
            status_filter(ko_filter, ok_filter, verbose);
            for (_pid, job) in cronjobs {
                // un row en 2 parties, avec une couleur qui change au milieu
                let mut start_of_row =
                    vec![Cell::new(&job.pid.to_string()), Cell::new(job.get_user())];
                match job.status {
                    JobStatus::Ok => start_of_row.push(Cell::new("OK").style_spec("Fg")),
                    JobStatus::Failed => start_of_row.push(Cell::new("KO").style_spec("bFr")),
                    JobStatus::Unknow => start_of_row.push(Cell::new("unknow").style_spec("Fb")),
                }
                let mut end_of_row = vec![
                    (Cell::new(job.get_dates().as_str())),
                    (Cell::new(job.get_duration().as_str())),
                    //(Cell::new(job.get_command().as_str())),
                    (Cell::new(textwrap::fill(job.get_command().as_str(), 80).as_str())),
                ];
                start_of_row.append(&mut end_of_row);
                table.add_row(Row::new(start_of_row));
            }
            table.printstd();
        }
    }
}

pub fn grep_file(config: Config, verbose: bool) -> Result<Vec<String>, Box<dyn Error>> {
    // lecture fichiers
    // utilisation de DecompressionReader pour gérer les .gz
    let mut reader = DecompressionReader::new(&config.filename)?;
    let mut contents = vec![];
    reader.read_to_end(&mut contents)?;

    // construction matcher depuis la regexp
    let query = format!("{}", config.query);
    let matcher = RegexMatcher::new(query.as_str())?;
    if verbose {
        println!("DEBUG: regex query : {}", query);
    }

    // search des matches dans contents
    // on met les lines dans un Vec qu'on retourne
    let mut matches: Vec<String> = vec![];
    Searcher::new().search_slice(
        &matcher,
        &contents,
        UTF8(|_lnum, line| {
            matches.push(line.to_string());
            Ok(true)
        }),
    )?;
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_file_result() {
        let filename = String::from("tests/sample_cron.log");
        let query = String::from("59697");
        let config = Config::new(&query, &filename).unwrap();
        let expected_res = Vec::from(["Mar 23 14:35:01 srv4 CRON[59697]: (_tuptime) CMD (   if [ -x /usr/bin/tuptime ]; then /usr/bin/tuptime -x > /dev/null; fi)\n".to_string()]);
        match grep_file(config, false) {
            Ok(res) => assert_eq!(expected_res, res),
            Err(e) => eprintln!("Application error: {}", e),
        };
    }
}
