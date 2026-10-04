// @feature usage-analytics
// @spec docs/features/usage-analytics.md

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptInvocation {
    pub path: String,
    pub interpreter: Option<String>,
}

pub(crate) fn detect_script_invocations(
    command: &str,
    project_root: &Path,
) -> Vec<ScriptInvocation> {
    ScriptDetector::new(project_root)
        .map(|mut detector| detector.detect(command))
        .unwrap_or_default()
}

struct ScriptDetector {
    project_root: PathBuf,
    working_directory: PathBuf,
    invocations: Vec<ScriptInvocation>,
}

impl ScriptDetector {
    fn new(project_root: &Path) -> Option<Self> {
        let project_root = project_root.canonicalize().ok()?;
        Some(Self {
            working_directory: project_root.clone(),
            project_root,
            invocations: Vec::new(),
        })
    }

    fn detect(&mut self, command: &str) -> Vec<ScriptInvocation> {
        for segment in shell_segments(command) {
            self.observe(segment);
        }
        std::mem::take(&mut self.invocations)
    }

    fn observe(&mut self, segment: ShellSegment) {
        let Ok(words) = shell_words::split(&segment.command) else {
            return;
        };
        let words = strip_assignments(&words);
        if words.first().is_some_and(|word| word == "cd") {
            self.change_directory(words.get(1), segment.separator);
        } else if let Some(invocation) =
            invocation_from_words(words, &self.working_directory, &self.project_root)
        {
            self.invocations.push(invocation);
        }
    }

    fn change_directory(&mut self, path: Option<&String>, separator: Separator) {
        if separator == Separator::Pipe {
            return;
        }
        let Some(directory) = path
            .and_then(|path| resolve_directory(&self.working_directory, &self.project_root, path))
        else {
            return;
        };
        self.working_directory = directory;
    }
}

fn invocation_from_words(
    words: &[String],
    working_directory: &Path,
    project_root: &Path,
) -> Option<ScriptInvocation> {
    let operand = script_operand(unwrap_command(words)?)?;
    resolve_script(
        operand.path,
        working_directory,
        project_root,
        operand.interpreter,
    )
}

struct ScriptOperand<'a> {
    path: &'a str,
    interpreter: Option<String>,
}

fn script_operand(words: &[String]) -> Option<ScriptOperand<'_>> {
    let (program, arguments) = words.split_first()?;
    let program_name = executable_name(program)?;
    if matches!(program_name, "source" | ".") {
        return source_operand(arguments);
    }
    if is_interpreter(program_name) {
        return interpreter_operand(arguments, program_name);
    }
    looks_like_path(program).then_some(ScriptOperand {
        path: program,
        interpreter: None,
    })
}

fn source_operand(arguments: &[String]) -> Option<ScriptOperand<'_>> {
    Some(ScriptOperand {
        path: arguments.first()?,
        interpreter: None,
    })
}

fn interpreter_operand<'a>(
    arguments: &'a [String],
    interpreter: &str,
) -> Option<ScriptOperand<'a>> {
    Some(ScriptOperand {
        path: interpreter_script(arguments)?,
        interpreter: Some(interpreter.to_owned()),
    })
}

fn unwrap_command(mut words: &[String]) -> Option<&[String]> {
    loop {
        match wrapper_step(words)? {
            WrapperStep::Command => return Some(words),
            WrapperStep::Nested(nested) => words = nested,
            WrapperStep::Ignored => return None,
        }
    }
}

enum WrapperStep<'a> {
    Command,
    Nested(&'a [String]),
    Ignored,
}

fn wrapper_step(words: &[String]) -> Option<WrapperStep<'_>> {
    let (program, arguments) = words.split_first()?;
    let program_name = executable_name(program)?;
    match program_name {
        "nexus" if arguments.first().is_some_and(|value| value == "exec") => {
            Some(WrapperStep::Ignored)
        }
        "env" => env_command(arguments).map(WrapperStep::Nested),
        "uv" if arguments.first().is_some_and(|value| value == "run") => {
            uv_command(arguments).map(WrapperStep::Nested)
        }
        "command" | "nohup" | "time" => launcher_command(arguments).map(WrapperStep::Nested),
        _ => Some(WrapperStep::Command),
    }
}

fn env_command(arguments: &[String]) -> Option<&[String]> {
    let index = arguments
        .iter()
        .position(|value| value == "--" || (!value.starts_with('-') && !is_assignment(value)))?;
    let start = index + usize::from(arguments[index] == "--");
    arguments.get(start..)
}

fn uv_command(arguments: &[String]) -> Option<&[String]> {
    let nested = arguments
        .get(1..)?
        .iter()
        .position(|value| !value.starts_with('-'))?
        + 1;
    arguments.get(nested..)
}

fn launcher_command(arguments: &[String]) -> Option<&[String]> {
    let nested = arguments.iter().position(|value| !value.starts_with('-'))?;
    arguments.get(nested..)
}

fn executable_name(program: &str) -> Option<&str> {
    Path::new(program).file_name()?.to_str()
}

fn interpreter_script(arguments: &[String]) -> Option<&str> {
    let mut index = 0;
    while let Some(argument) = arguments.get(index) {
        if argument == "--" {
            return arguments.get(index + 1).map(String::as_str);
        }
        if matches!(argument.as_str(), "-c" | "-e" | "--eval" | "-m") {
            return None;
        }
        if argument.starts_with('-') {
            index += 1;
            continue;
        }
        return Some(argument);
    }
    None
}

fn resolve_script(
    path: &str,
    working_directory: &Path,
    project_root: &Path,
    interpreter: Option<String>,
) -> Option<ScriptInvocation> {
    let candidate = resolve(working_directory, path).canonicalize().ok()?;
    if !candidate.is_file() || !candidate.starts_with(project_root) {
        return None;
    }
    let relative = candidate.strip_prefix(project_root).ok()?;
    Some(ScriptInvocation {
        path: relative.to_string_lossy().replace('\\', "/"),
        interpreter,
    })
}

fn resolve_directory(working_directory: &Path, project_root: &Path, path: &str) -> Option<PathBuf> {
    let directory = resolve(working_directory, path).canonicalize().ok()?;
    (directory.is_dir() && directory.starts_with(project_root)).then_some(directory)
}

fn resolve(working_directory: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_directory.join(path)
    }
}

fn looks_like_path(value: &str) -> bool {
    value.contains('/') || value.starts_with('.')
}

fn is_interpreter(value: &str) -> bool {
    matches!(
        value,
        "bash" | "sh" | "zsh" | "fish" | "node" | "ruby" | "perl"
    ) || value == "python"
        || value.strip_prefix("python").is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .chars()
                    .all(|item| item.is_ascii_digit() || item == '.')
        })
}

fn strip_assignments(words: &[String]) -> &[String] {
    let start = words
        .iter()
        .position(|word| !is_assignment(word))
        .unwrap_or(words.len());
    &words[start..]
}

fn is_assignment(value: &str) -> bool {
    let Some((name, _)) = value.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name.chars().enumerate().all(|(index, character)| {
            character == '_'
                || character.is_ascii_alphanumeric() && (index > 0 || !character.is_ascii_digit())
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Separator {
    End,
    Sequence,
    Pipe,
}

struct ShellSegment {
    command: String,
    separator: Separator,
}

fn shell_segments(command: &str) -> Vec<ShellSegment> {
    let mut scanner = ShellScanner::default();
    let mut characters = command.chars().peekable();
    while let Some(character) = characters.next() {
        if scanner.push(character, characters.peek().copied()) {
            characters.next();
        }
    }
    scanner.finish()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Quote {
    #[default]
    None,
    Single,
    Double,
}

#[derive(Default)]
struct ShellScanner {
    segments: Vec<ShellSegment>,
    current: String,
    quote: Quote,
    escaped: bool,
}

impl ShellScanner {
    fn push(&mut self, character: char, next: Option<char>) -> bool {
        if self.push_quoted_or_escaped(character) {
            return false;
        }
        let Some((separator, consume_next)) = shell_separator(character, next) else {
            self.current.push(character);
            return false;
        };
        self.close_segment(separator);
        consume_next
    }

    fn push_quoted_or_escaped(&mut self, character: char) -> bool {
        if self.escaped {
            self.current.push(character);
            self.escaped = false;
            return true;
        }
        if character == '\\' && self.quote != Quote::Single {
            self.current.push(character);
            self.escaped = true;
            return true;
        }
        if let Some(quote) = toggled_quote(self.quote, character) {
            self.quote = quote;
            self.current.push(character);
            return true;
        }
        if self.quote != Quote::None {
            self.current.push(character);
            return true;
        }
        false
    }

    fn close_segment(&mut self, separator: Separator) {
        if !self.current.trim().is_empty() {
            self.segments.push(ShellSegment {
                command: self.current.trim().to_owned(),
                separator,
            });
        }
        self.current.clear();
    }

    fn finish(mut self) -> Vec<ShellSegment> {
        self.close_segment(Separator::End);
        self.segments
    }
}

fn toggled_quote(quote: Quote, character: char) -> Option<Quote> {
    match (quote, character) {
        (Quote::None, '\'') => Some(Quote::Single),
        (Quote::Single, '\'') => Some(Quote::None),
        (Quote::None, '"') => Some(Quote::Double),
        (Quote::Double, '"') => Some(Quote::None),
        _ => None,
    }
}

fn shell_separator(character: char, next: Option<char>) -> Option<(Separator, bool)> {
    match (character, next) {
        (';' | '\n', _) => Some((Separator::Sequence, false)),
        ('|', Some('|')) | ('&', Some('&')) => Some((Separator::Sequence, true)),
        ('|', _) => Some((Separator::Pipe, false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_repository_scripts_across_shell_forms_without_wrapper_duplicates() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::create_dir_all(root.join("tools")).unwrap();
        std::fs::write(root.join("scripts/check.sh"), "#!/bin/sh\n").unwrap();
        std::fs::write(root.join("tools/report.py"), "print('ok')\n").unwrap();

        let detected = detect_script_invocations(
            "./scripts/check.sh && cd tools && python3 report.py | tee output.txt",
            root,
        );
        assert_eq!(
            detected
                .iter()
                .map(|item| (item.path.as_str(), item.interpreter.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("scripts/check.sh", None),
                ("tools/report.py", Some("python3")),
            ]
        );

        assert!(
            detect_script_invocations("nexus exec -- ./scripts/check.sh --token secret", root,)
                .is_empty()
        );
        assert!(detect_script_invocations("python3 -c 'print(1)'", root).is_empty());
    }

    #[test]
    fn unwraps_launchers_and_keeps_quoted_separators_inside_script_paths() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::create_dir_all(root.join("tools")).unwrap();
        std::fs::write(root.join("scripts/check.sh"), "#!/bin/sh\n").unwrap();
        std::fs::write(root.join("scripts/semi;colon.sh"), "#!/bin/sh\n").unwrap();
        std::fs::write(root.join("tools/report.py"), "print('ok')\n").unwrap();

        let detected = detect_script_invocations(
            "env TOKEN=private -- command python3 tools/report.py; \
             uv run bash scripts/check.sh; source 'scripts/semi;colon.sh'",
            root,
        );

        assert_eq!(
            detected,
            vec![
                ScriptInvocation {
                    path: "tools/report.py".to_owned(),
                    interpreter: Some("python3".to_owned()),
                },
                ScriptInvocation {
                    path: "scripts/check.sh".to_owned(),
                    interpreter: Some("bash".to_owned()),
                },
                ScriptInvocation {
                    path: "scripts/semi;colon.sh".to_owned(),
                    interpreter: None,
                },
            ]
        );
    }

    #[test]
    fn piped_cd_does_not_change_the_following_working_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::create_dir_all(root.join("tools")).unwrap();
        std::fs::write(root.join("scripts/check.sh"), "#!/bin/sh\n").unwrap();

        let detected = detect_script_invocations("cd tools | cat; ./scripts/check.sh", root);

        assert_eq!(
            detected,
            vec![ScriptInvocation {
                path: "scripts/check.sh".to_owned(),
                interpreter: None,
            }]
        );
    }
}
