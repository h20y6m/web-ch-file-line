use std::collections::VecDeque;
use std::error::Error;
use std::fs;
use std::io::prelude::*;
use std::io::{BufReader, BufWriter};
use std::rc::Rc;

pub struct Config {
    verbose: usize,
    directory: Option<String>,
    output: Option<String>,
    webfilename: String,
    chfilenames: Vec<String>,
}

impl Config {
    pub fn new(args: &[String]) -> Result<Config, &'static str> {
        let mut directory = None;
        let mut output = None;
        let mut verbose = 0;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-v" => {
                    verbose += 1;
                    i += 1;
                }
                "-vv" => {
                    verbose += 2;
                    i += 1;
                }
                "-w" => {
                    if i + 1 < args.len() {
                        directory = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        return Err("-w require working directory");
                    }
                }
                "-o" => {
                    if i + 1 < args.len() {
                        output = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        return Err("-o require output filename");
                    }
                }
                "--" => {
                    i += 1;
                    break;
                }
                _ => break,
            }
        }

        if args.len() - i < 2 {
            return Err("not enough arguments");
        }

        let webfilename = args[i].clone();
        i += 1;
        let chfilenames = args[i..].iter().cloned().collect();

        Ok(Config {
            verbose,
            directory,
            output,
            webfilename,
            chfilenames,
        })
    }
}

pub fn run(config: Config) -> Result<(), Box<dyn Error>> {
    if let Some(working_directory) = config.directory {
        if config.verbose > 0 {
            eprintln!("Working directory: {}", &working_directory);
        }
        std::env::set_current_dir(&working_directory)?;
    }

    if config.verbose > 0 {
        eprintln!("Web file: {}", &config.webfilename);
    }
    let mut weblines = read_filelines(&config.webfilename)?;

    for chfilename in &config.chfilenames {
        if config.verbose > 0 {
            eprintln!("Change file: {}", &chfilename);
        }
        let chfile = read_changefile(chfilename)?;

        weblines = apply_changefile(weblines, chfile)?;
    }

    match config.output.as_deref() {
        None => {
            print_filelines(&weblines);
        }
        Some("-") => {
            write_filelines(std::io::stdout(), &weblines)?;
        }
        Some(output) => {
            if config.verbose > 0 {
                eprintln!("Output file: {}", &output);
            }
            write_filelines(BufWriter::new(fs::File::create(output)?), &weblines)?;
        }
    }

    Ok(())
}

fn print_filelines(lines: &[FileLine]) {
    let max_filename = lines
        .iter()
        .fold(0, |max, line| usize::max(max, line.filename.len()));
    let max_line_str = usize_str_len(
        lines
            .iter()
            .fold(0, |max, line| usize::max(max, line.line_num)),
    );
    let width = max_filename + max_line_str + 2;

    for line in lines {
        let fileline = format!("{}({})", line.filename, line.line_num);
        print!("{:width$} | ", fileline, width = width);
        for &b in &line.contents {
            if b >= 0x20 && b <= 0x7E {
                print!("{}", char::from(b));
            } else {
                print!("\x1B[7m<{:02X}>\x1B[0m", b);
            }
        }
        println!();
    }
}

fn write_filelines<W: Write>(mut w: W, lines: &[FileLine]) -> std::io::Result<()> {
    let max_filename = lines
        .iter()
        .fold(0, |max, line| usize::max(max, line.filename.len()));
    let max_line_str = usize_str_len(
        lines
            .iter()
            .fold(0, |max, line| usize::max(max, line.line_num)),
    );
    let width = max_filename + max_line_str + 2;

    for line in lines {
        let fileline = format!("{}({})", line.filename, line.line_num);
        write!(w, "{:width$} | ", fileline, width = width)?;
        w.write_all(&line.contents)?;
        if cfg!(windows) {
            w.write(&[0x0D, 0x0A])?;
        } else {
            w.write(&[0x0A])?;
        }
    }
    w.flush()?;
    Ok(())
}

fn usize_str_len(n: usize) -> usize {
    let base = 10;
    let mut power = base;
    let mut count = 1;
    while n >= power {
        count += 1;
        if let Some(new_power) = power.checked_mul(base) {
            power = new_power;
        } else {
            break;
        }
    }
    count
}

struct ChangeFileSection {
    headline: FileLine,
    oldlines: Vec<FileLine>,
    newlines: Vec<FileLine>,
}

fn read_changefile(filename: &str) -> Result<Vec<ChangeFileSection>, Box<dyn Error>> {
    let mut chfilelines = read_filelines(filename)?.into_iter();

    let mut sections = Vec::new();
    'outer: loop {
        // find line begin `@x`.
        let headline;
        loop {
            let line = if let Some(line) = chfilelines.next() {
                line
            } else {
                break 'outer;
            };

            if line.contents.starts_with(b"@x") {
                headline = Some(line);
                break;
            }
            if line.contents.starts_with(b"@y") || line.contents.starts_with(b"@z") {
                return Err(format!(
                    "Change file missing @x at {}({})",
                    line.filename, line.line_num
                )
                .into());
            }
        }
        let headline = headline.unwrap();

        // find line begin `@y`.
        let mut oldlines = Vec::new();
        loop {
            let line = if let Some(line) = chfilelines.next() {
                line
            } else {
                return Err(format!(
                    "Change file ended after @x at {}({})",
                    headline.filename, headline.line_num
                )
                .into());
            };

            if line.contents.starts_with(b"@y") {
                break;
            }

            if oldlines.len() > 0 || u8_slice_trim_start(&line.contents).len() > 0 {
                oldlines.push(line);
            }
        }

        // find line begin `@z`.
        let mut newlines = Vec::new();
        loop {
            let line = if let Some(line) = chfilelines.next() {
                line
            } else {
                eprintln!("At the end of change file missing @z [{}]", filename);
                break;
            };

            if line.contents.starts_with(b"@z") {
                break;
            }

            newlines.push(line);
        }

        sections.push(ChangeFileSection {
            headline,
            oldlines,
            newlines,
        })
    }

    Ok(sections)
}

fn apply_changefile(
    weblines: Vec<FileLine>,
    chfilesections: Vec<ChangeFileSection>,
) -> Result<Vec<FileLine>, Box<dyn Error>> {
    let mut result = Vec::new();
    let mut weblines = VecDeque::from(weblines);

    fn match_position(weblines: &VecDeque<FileLine>, oldlines: &Vec<FileLine>) -> Option<usize> {
        if weblines.len() < oldlines.len() {
            return None;
        }
        for i in 0..weblines.len() {
            if weblines
                .range(i..)
                .take(oldlines.len())
                .map(|l| &l.contents)
                .eq(oldlines.iter().map(|l| &l.contents))
            {
                return Some(i);
            }
        }
        None
    }

    for mut section in chfilesections {
        if let Some(pos) = match_position(&weblines, &section.oldlines) {
            result.reserve(pos + section.newlines.len());
            for _ in 0..pos {
                result.push(weblines.pop_front().unwrap());
            }
            for _ in 0..section.oldlines.len() {
                weblines.pop_front();
            }
            result.append(&mut section.newlines);
        } else {
            return Err(format!(
                "Change file section do not match [{}({})]",
                section.headline.filename, section.headline.line_num,
            )
            .into());
        }
    }
    result.reserve(weblines.len());
    for line in weblines {
        result.push(line);
    }

    Ok(result)
}

fn u8_slice_trim_start(s: &[u8]) -> &[u8] {
    let first = s
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(s.len());
    &s[first..]
}

struct FileLine {
    filename: Rc<String>,
    line_num: usize,
    contents: Vec<u8>,
}

fn read_filelines(filename: &str) -> Result<Vec<FileLine>, Box<dyn Error>> {
    let filename = Rc::new(filename.to_string());

    let mut f = BufReader::new(fs::File::open(filename.as_str())?);

    let mut filelines = Vec::new();
    for line_num in 1.. {
        let mut contents = Vec::new();

        // read line delimited by LF
        if f.read_until(0x0A, &mut contents)? == 0 {
            break;
        }

        // remove tail LF
        if Some(&0x0A) == contents.last() {
            contents.pop();

            // if windows, also remove CR.
            if cfg!(windows) && Some(&0x0D) == contents.last() {
                contents.pop();
            }
        }

        filelines.push(FileLine {
            filename: Rc::clone(&filename),
            line_num,
            contents,
        })
    }

    Ok(filelines)
}
