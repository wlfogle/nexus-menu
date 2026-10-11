//! The few command-line options the app takes, the same ones the command
//! line has, so the window can be pointed at a scratch directory or an extra
//! Wine prefix the same way.

use std::path::PathBuf;

use nexus_menu::engine::Options;

pub const USAGE: &str = "\
Usage: nexus-menu-gui [OPTIONS]

Options:
      --dir <DIR>        Directory to write generated entries into
                         [default: $XDG_DATA_HOME/applications]
      --catalog <FILE>   Extra catalog file to overlay on the built-in one
                         [default: $XDG_CONFIG_HOME/nexus-menu/catalog.toml]
      --prefix <DIR>     Extra Wine prefix, or a directory to search for them
                         (repeatable)
      --ignore-nodisplay Treat NoDisplay/Hidden entries as missing
  -V, --version          Print the version
  -h, --help             Print this help
";

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    Version,
    Help,
}

/// Parse `args` (without the program name).
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut out = Options::default();

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| {
            args.next()
                .ok_or_else(|| format!("{flag} needs a value"))
                .map(PathBuf::from)
        };
        match arg.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-V" | "--version" => return Ok(Parsed::Version),
            "--ignore-nodisplay" => out.ignore_nodisplay = true,
            "--dir" => out.target_dir = value("--dir")?,
            "--catalog" => out.catalog_path = value("--catalog")?,
            "--prefix" => out.extra_prefixes.push(value("--prefix")?),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(Parsed::Run(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Parsed, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    fn opts(args: &[&str]) -> Options {
        match run(args).unwrap() {
            Parsed::Run(o) => o,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn no_arguments_gives_the_command_line_defaults() {
        assert_eq!(opts(&[]), Options::default());
    }

    #[test]
    fn flags_take_values_and_prefix_repeats() {
        let o = opts(&[
            "--dir",
            "/tmp/apps",
            "--catalog",
            "/tmp/c.toml",
            "--prefix",
            "/a",
            "--prefix",
            "/b",
            "--ignore-nodisplay",
        ]);
        assert_eq!(o.target_dir, PathBuf::from("/tmp/apps"));
        assert_eq!(o.catalog_path, PathBuf::from("/tmp/c.toml"));
        assert_eq!(
            o.extra_prefixes,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert!(o.ignore_nodisplay);
    }

    #[test]
    fn help_and_version_win() {
        assert_eq!(run(&["--help"]), Ok(Parsed::Help));
        assert_eq!(run(&["-h"]), Ok(Parsed::Help));
        assert_eq!(run(&["--version"]), Ok(Parsed::Version));
        assert_eq!(run(&["-V"]), Ok(Parsed::Version));
    }

    #[test]
    fn a_missing_value_or_unknown_flag_is_an_error() {
        assert_eq!(run(&["--dir"]), Err("--dir needs a value".to_string()));
        assert_eq!(
            run(&["--prefix"]),
            Err("--prefix needs a value".to_string())
        );
        assert_eq!(
            run(&["--nope"]),
            Err("unknown argument `--nope`".to_string())
        );
    }
}
