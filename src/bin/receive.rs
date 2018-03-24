#![feature(termination_trait, process_exitcode_placeholder)]

extern crate integer_encoding;
extern crate crc;
extern crate byteorder;
#[macro_use]
extern crate failure;
extern crate tempdir;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_yaml;

use byteorder::{BigEndian, ReadBytesExt};
use crc::{crc32, Hasher32};
use integer_encoding::VarIntReader;
use tempdir::TempDir;

use std::borrow::Cow;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T> = std::result::Result<T, failure::Error>;

fn no_command() {
  eprintln!("Howdy. This is Travis SSH Deploy.");
  eprintln!("You can't SSH in to a shell, but you can provide me with a command!");
  eprintln!("Check the documentation for more information.");
  eprintln!();
  eprintln!("Bye!");
}

fn main() -> ExitCode {
  let config_path = match env::args().nth(1) {
    Some(c) => c,
    None => {
      eprintln!("{}! Travis SSH Deploy isn't set up right.", expletive());
      eprintln!("I don't know where to find the config file!");
      return ExitCode::FAILURE;
    }
  };

  let f = match File::open(config_path) {
    Ok(f) => f,
    Err(e) => {
      eprintln!("{}! I couldn't open up the config file. Here's what I know:", expletive());
      eprintln!("{}", e);
      return ExitCode::FAILURE;
    }
  };

  let config: Config = match serde_yaml::from_reader(f) {
    Ok(c) => c,
    Err(e) => {
      eprintln!("{}! I couldn't parse the config file. Here's what I know:", expletive());
      eprintln!("{}", e);
      return ExitCode::FAILURE;
    }
  };

  let ssh_command = match env::var("SSH_ORIGINAL_COMMAND") {
    Ok(c) => c,
    Err(_) => {
      no_command();
      return ExitCode::FAILURE;
    }
  };

  let parts: Vec<&str> = ssh_command
    .split(' ')
    .filter(|x| !x.is_empty())
    .collect();
  if parts.is_empty() {
    no_command();
    return ExitCode::FAILURE;
  }
  let command = &parts[0];
  let params = &parts[1..];

  let res = match *command {
    "deploy" => deploy(&config, &params),
    x => {
      eprintln!("{}! I'm not sure what to do with \"{}\"", expletive(), x);
      return ExitCode::FAILURE;
    }
  };

  if let Err(e) = res {
    eprintln!("{}! Something went wrong. Here's what I know.", expletive());
    eprintln!("{}", e);
    return ExitCode::FAILURE;
  }

  ExitCode::SUCCESS
}

fn deploy(config: &Config, params: &[&str]) -> Result<()> {
  println!("Executing a deploy plan!");

  if params.is_empty() {
    eprintln!("{}! You didn't tell me what plan we're working on!", expletive());
    eprintln!("Usage: deploy [plan]");
    return Ok(());
  }

  let plan = match config.plans.get(params[0]) {
    Some(p) => p,
    None => {
      eprintln!("{}! I couldn't find any plan called {}. :(", expletive(),params[0]);
      return Ok(());
    }
  };

  println!("Good news! I found your deploy plan.");

  let mut state = PlanState::new(plan);

  println!("Now I'll just go through the motions...");
  println!();

  for (i, step) in plan.steps.iter().enumerate() {
    println!("Step {}: {}", i + 1, step.describe());
    step.execute(&mut state)?;
    println!();
  }

  println!("Finished! See ya!");

  Ok(())
}

fn receive_files(state: &mut PlanState) -> Result<()> {
  let mut stdin = io::stdin();
  let mut buf = [0; 512];

  stdin.read_exact(&mut buf[..5])?;

  if buf[..4] != [0xFE, 0xED, 0xBE, 0xEF] {
    bail!("Bad protocol magic bytes.");
  }

  if buf[4] != 0x02 {
    bail!("Bad protocol version.");
  }

  let num_files: usize = stdin.read_varint()?;

  let exp_files = match state.plan.expected_files {
    Some(ExpectedFiles::Amount(a)) => Some(a),
    Some(ExpectedFiles::List(ref l)) => Some(l.len()),
    None => None
  };
  if let Some(a) = exp_files {
    if num_files != a {
      bail!("Not enough files sent for this plan.");
    }
  }

  let tmp = TempDir::new("travis-deploy")?;
  let mut files = Vec::with_capacity(num_files);

  for _ in 0..num_files {
    stdin.read_exact(&mut buf[..1])?;
    let compression = buf[0];
    if compression != 0 {
      bail!("Compression isn't supported just yet.");
    }

    let name_length: usize = stdin.read_varint()?;
    if name_length > 512 {
      bail!("File names longer than 512 bytes aren't allowed.");
    }
    stdin.read_exact(&mut buf[..name_length])?;
    let file_name = String::from_utf8(buf[..name_length].to_vec())?;

    let crc = stdin.read_u32::<BigEndian>()?;

    let file_len: usize = stdin.read_varint()?;
    if file_len > 104_857_600 {
      bail!("Files larger than 100 MiB aren't allowed.");
    }

    let mut f = BufWriter::new(File::create(tmp.path().join(&file_name))?);

    let mut total = 0;
    let mut in_crc = crc32::Digest::new(crc32::IEEE);

    while total < file_len {
      let left = file_len - total;
      let read = if left < 512 {
        stdin.read_exact(&mut buf[..left])?;
        left
      } else {
        stdin.read(&mut buf)?
      };
      if read == 0 {
        bail!("Input ended before I could read {} bytes for {}.", file_len, file_name);
      }
      total += read;
      in_crc.write(&buf[..read]);
      f.write_all(&buf[..read])?;
    }

    if in_crc.sum32() != crc {
      bail!("CRCs did not match for {}.", file_name);
    }

    println!("Successfully received {}.", file_name);

    files.push(file_name);
  }

  if let Some(ExpectedFiles::List(ref exp))  = state.plan.expected_files {
    let mut files = files.clone();
    files.sort();
    let mut exp = exp.clone();
    exp.sort();

    if files != exp {
      bail!("Expected list of files did not match uploaded files.");
    }
  }

  state.tmp = Some(tmp);
  state.files = Some(files);

  Ok(())
}

fn move_files(state: &mut PlanState) -> Result<()> {
  let tmp = match state.tmp {
    Some(ref t) => t,
    None => bail!("move_files cannot be run before receive_files (missing tmp)")
  };
  let files = match state.files {
    Some(ref f) => f,
    None => bail!("move_files cannot be run before receive_files (missing files)")
  };

  for name in files {
    let dest = match state.plan.files {
      Some(ref f) => f.get(name).unwrap_or(name),
      None => name
    };
    let dest = match state.plan.working_directory {
      Some(ref workdir) => Path::new(workdir).join(dest),
      None => PathBuf::from(dest)
    };
    println!("Renaming {} to {}", tmp.path().join(name).to_string_lossy(), dest.to_string_lossy());
    fs::rename(
      tmp.path().join(name),
      dest
    )?;
  }
  Ok(())
}

fn command(state: &mut PlanState, cc: &ConfigCommand) -> Result<()> {
  let mut command = Command::new(&cc.command);

  if let Some(ref args) = cc.args {
    command.args(args);
  }
  if let Some(ref workdir) = state.plan.working_directory {
    command.current_dir(workdir);
  }
  if let Some(ref tmp) = state.tmp {
    command.env("TRAVIS_DEPLOY_TEMPDIR", tmp.path().to_string_lossy().to_string());
  }
  if let Ok(path) = env::var("PATH") {
    command.env("PATH", path);
  }

  if !command.status()?.success() && !cc.allow_failure {
    bail!("Command {} exited with a non-zero status code", cc.command);
  }

  Ok(())
}

#[derive(Deserialize)]
pub struct Config {
  plans: HashMap<String, Plan>,
}

#[derive(Deserialize)]
pub struct Plan {
  working_directory: Option<String>,
  expected_files: Option<ExpectedFiles>,
  files: Option<HashMap<String, String>>,
  steps: Vec<Step>,
}

#[derive(Deserialize)]
pub struct ConfigCommand {
  command: String,
  args: Option<Vec<String>>,
  #[serde(default)]
  allow_failure: bool,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
  Command(ConfigCommand),
  ReceiveFiles,
  MoveFiles,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub enum ExpectedFiles {
  Amount(usize),
  List(Vec<String>)
}

struct PlanState<'a> {
  plan: &'a Plan,
  tmp: Option<TempDir>,
  files: Option<Vec<String>>
}

impl<'a> PlanState<'a> {
  fn new(plan: &'a Plan) -> Self {
    PlanState {
      plan,
      tmp: None,
      files: None
    }
  }
}

impl Step {
  fn execute(&self, state: &mut PlanState) -> Result<()> {
    match *self {
      Step::ReceiveFiles => receive_files(state),
      Step::MoveFiles => move_files(state),
      Step::Command(ref cc) => command(state, cc),
    }
  }

  fn describe(&self) -> Cow<str> {
    match *self {
      Step::ReceiveFiles => Cow::Borrowed("receive files"),
      Step::MoveFiles => Cow::Borrowed("move files"),
      Step::Command(ref cc) => Cow::Owned(format!(
        "run command `{}{}`",
        cc.command,
        cc.args.as_ref().map(|x| format!(" {}", x.join(" "))).unwrap_or_default()
      ))
    }
  }
}
