//! Bounded declarative command parsing. Applications own their command names,
//! field mapping, and effect policy; Clap is a private implementation detail.

use std::collections::BTreeMap;
use std::ffi::OsStr;

/// Maximum number of command-line words accepted by [`Command::parse`].
pub const MAX_ARGUMENTS: usize = 1_024;
/// Maximum UTF-8 bytes accepted for one command-line word.
pub const MAX_ARGUMENT_BYTES: usize = 64 * 1024;
/// Maximum total UTF-8 bytes accepted for all command-line words.
pub const MAX_TOTAL_ARGUMENT_BYTES: usize = 1024 * 1024;

/// A facade-owned scalar validation rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueKind {
    /// Any non-NUL UTF-8 string within the input limits.
    String,
    /// One of the declared strings.
    Enumeration(Vec<String>),
}

impl ValueKind {
    /// Accept an arbitrary string value.
    pub const fn string() -> Self {
        Self::String
    }

    /// Accept exactly one of `values`.
    pub fn enumeration<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Enumeration(values.into_iter().map(Into::into).collect())
    }
}

/// One long option in a [`Command`] schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionSpec {
    name: String,
    kind: Option<ValueKind>,
    default: Option<String>,
}

impl OptionSpec {
    /// Declare a boolean `--name` flag.
    pub fn flag(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: None,
            default: None,
        }
    }

    /// Declare a string-valued `--name VALUE` option.
    pub fn value(name: impl Into<String>, kind: ValueKind) -> Self {
        Self {
            name: name.into(),
            kind: Some(kind),
            default: None,
        }
    }

    /// Supply a value used when this option is absent.
    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.default = Some(value.into());
        self
    }
}

/// A declarative command and its nested subcommands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Command {
    name: String,
    options: Vec<OptionSpec>,
    subcommands: Vec<Self>,
}

impl Command {
    /// Start a command schema.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            options: Vec::new(),
            subcommands: Vec::new(),
        }
    }

    /// Add a long option.
    pub fn option(mut self, option: OptionSpec) -> Self {
        self.options.push(option);
        self
    }

    /// Add a nested subcommand.
    pub fn subcommand(mut self, command: Self) -> Self {
        self.subcommands.push(command);
        self
    }

    /// Parse one bounded command line into facade-owned values.
    ///
    /// The private parser never runs a shell. Input limits are checked before
    /// it receives any owned words; errors deliberately do not echo input.
    pub fn parse<I, S>(&self, arguments: I) -> Result<ParsedCommand, CommandError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut total = 0usize;
        let mut words = Vec::new();
        for argument in arguments {
            if words.len() == MAX_ARGUMENTS {
                return Err(CommandError::TooManyArguments);
            }
            let value = argument
                .as_ref()
                .to_str()
                .ok_or(CommandError::InvalidUtf8)?;
            if value.contains('\0') {
                return Err(CommandError::ContainsNul);
            }
            if value.len() > MAX_ARGUMENT_BYTES {
                return Err(CommandError::ArgumentTooLarge);
            }
            total = total
                .checked_add(value.len())
                .ok_or(CommandError::InputTooLarge)?;
            if total > MAX_TOTAL_ARGUMENT_BYTES {
                return Err(CommandError::InputTooLarge);
            }
            words.push(value.to_owned());
        }
        self.validate()?;
        let command = self.clap_command();
        let matches = command
            .try_get_matches_from(words)
            .map_err(|_| CommandError::InvalidArguments)?;
        let mut path = vec![self.name.clone()];
        let mut schema = self;
        let mut selected = vec![self];
        let mut final_matches = &matches;
        while let Some((name, next)) = final_matches.subcommand() {
            let Some(next_schema) = schema.subcommands.iter().find(|child| child.name == name)
            else {
                return Err(CommandError::InvalidArguments);
            };
            path.push(name.to_owned());
            schema = next_schema;
            selected.push(schema);
            final_matches = next;
        }
        let mut values = BTreeMap::new();
        for command in selected {
            command.collect_values(final_matches, &mut values)?;
        }
        Ok(ParsedCommand { path, values })
    }

    fn clap_command(&self) -> clap::Command {
        let mut command = clap::Command::new(self.name.clone())
            .disable_help_flag(false)
            .disable_version_flag(true)
            .disable_help_subcommand(true);
        for option in &self.options {
            let mut argument = clap::Arg::new(option.name.clone())
                .long(option.name.clone())
                .global(true);
            match &option.kind {
                None => argument = argument.action(clap::ArgAction::SetTrue),
                Some(ValueKind::String) => argument = argument.action(clap::ArgAction::Set),
                Some(ValueKind::Enumeration(values)) => {
                    argument = argument
                        .action(clap::ArgAction::Set)
                        .value_parser(values.clone());
                }
            }
            if let Some(default) = &option.default {
                argument = argument.default_value(default);
            }
            command = command.arg(argument);
        }
        for child in &self.subcommands {
            command = command.subcommand(child.clap_command());
        }
        command
    }

    fn validate(&self) -> Result<(), CommandError> {
        let mut option_names = std::collections::BTreeSet::new();
        self.validate_into(&mut option_names)
    }

    fn validate_into(
        &self,
        option_names: &mut std::collections::BTreeSet<String>,
    ) -> Result<(), CommandError> {
        if !valid_name(&self.name) || self.name == "help" {
            return Err(CommandError::InvalidSchema);
        }
        let mut child_names = std::collections::BTreeSet::new();
        for option in &self.options {
            if !valid_name(&option.name)
                || option.name == "help"
                || !option_names.insert(option.name.clone())
            {
                return Err(CommandError::InvalidSchema);
            }
            match (&option.kind, &option.default) {
                (None, Some(_)) => return Err(CommandError::InvalidSchema),
                (Some(ValueKind::Enumeration(values)), default)
                    if values.is_empty()
                        || values.iter().any(|value| value.contains('\0'))
                        || default
                            .as_ref()
                            .is_some_and(|value| !values.contains(value)) =>
                {
                    return Err(CommandError::InvalidSchema);
                }
                (_, Some(value)) if value.contains('\0') => {
                    return Err(CommandError::InvalidSchema)
                }
                _ => {}
            }
        }
        for child in &self.subcommands {
            if child.name == "help" || !child_names.insert(child.name.clone()) {
                return Err(CommandError::InvalidSchema);
            }
            child.validate_into(option_names)?;
        }
        Ok(())
    }

    fn collect_values(
        &self,
        matches: &clap::ArgMatches,
        values: &mut BTreeMap<String, ParsedValue>,
    ) -> Result<(), CommandError> {
        for option in &self.options {
            let value = match option.kind {
                None => ParsedValue::Flag(
                    matches
                        .try_get_one::<bool>(&option.name)
                        .map_err(|_| CommandError::InvalidArguments)?
                        .copied()
                        .unwrap_or(false),
                ),
                Some(_) => matches
                    .try_get_one::<String>(&option.name)
                    .map_err(|_| CommandError::InvalidArguments)?
                    .cloned()
                    .map(ParsedValue::String)
                    .unwrap_or(ParsedValue::Absent),
            };
            values.insert(option.name.clone(), value);
        }
        Ok(())
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Parsed scalar values independent of the private parser backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParsedValue {
    /// A declared flag.
    Flag(bool),
    /// A declared value option.
    String(String),
    /// A declared value option was absent and has no default.
    Absent,
}

/// A facade-owned parsed command line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedCommand {
    path: Vec<String>,
    values: BTreeMap<String, ParsedValue>,
}

impl ParsedCommand {
    /// Selected command names including the root.
    pub fn command_path(&self) -> &[String] {
        &self.path
    }

    /// Read a declared boolean flag.
    pub fn flag(&self, name: &str) -> Option<bool> {
        match self.values.get(name) {
            Some(ParsedValue::Flag(value)) => Some(*value),
            _ => None,
        }
    }

    /// Read a declared string option or its default.
    pub fn value(&self, name: &str) -> Option<&str> {
        match self.values.get(name) {
            Some(ParsedValue::String(value)) => Some(value),
            _ => None,
        }
    }
}

/// Parsing or schema failures without backend diagnostics or input echo.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CommandError {
    #[error("command schema is invalid")]
    InvalidSchema,
    #[error("command line contains a NUL character")]
    ContainsNul,
    #[error("command line contains a non-UTF-8 argument")]
    InvalidUtf8,
    #[error("command line has too many arguments")]
    TooManyArguments,
    #[error("command-line argument exceeds byte limit")]
    ArgumentTooLarge,
    #[error("command line exceeds byte limit")]
    InputTooLarge,
    #[error("invalid command-line arguments")]
    InvalidArguments,
}
