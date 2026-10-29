//! Default worktree branch names: `coocoo/<words>-<id>`, the words picked by omp's `smol` model
//! for the task, else taken from the agent's title.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

/// Length cap of the words part of a default worktree branch.
pub(crate) const BRANCH_TITLE_CHARS: usize = 40;
/// How long the model gets before the title names the branch instead.
const SUGGEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Task text the model sees.
const SUGGEST_INPUT_CHARS: usize = 2000;
/// A longer answer is chatter, not a name.
const SUGGEST_MAX_WORDS: usize = 6;

const SYSTEM_PROMPT: &str = "You name git branches. You never perform tasks and never use tools.";
const INSTRUCTION: &str = "Name a git branch for the task below. Do not work on the task. Reply with only the name: \
2 to 5 lowercase English words joined by hyphens that describe the change, no prefix, quotes or explanation.";

/// Default worktree branch: `coocoo/<slug>-<first 4 id chars>`, e.g.
/// `coocoo/git-status-push-pr-1a2b` (lowercase ASCII words, German umlauts transliterated, at most
/// [`BRANCH_TITLE_CHARS`]); `coocoo/<id>` without words or with nothing usable in them.
pub(crate) fn worktree_branch(words: Option<&str>, id: &str) -> String {
    let mut slug = String::new();
    for word in ascii_words(words.unwrap_or_default()).split_whitespace() {
        if !slug.is_empty() {
            if slug.len() + 1 + word.len() > BRANCH_TITLE_CHARS {
                break;
            }
            slug.push('-');
        }
        slug.push_str(word);
    }
    slug.truncate(BRANCH_TITLE_CHARS);
    if slug.is_empty() {
        return format!("coocoo/{id}");
    }
    format!("coocoo/{slug}-{}", &id[..id.len().min(4)])
}

/// Lowercase ASCII alphanumerics and spaces (ä/ö/ü/ß transliterated, everything else a space).
fn ascii_words(text: &str) -> String {
    let mut ascii = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => ascii.push_str("ae"),
            'ö' => ascii.push_str("oe"),
            'ü' => ascii.push_str("ue"),
            'ß' => ascii.push_str("ss"),
            c if c.is_ascii_alphanumeric() => ascii.push(c),
            _ => ascii.push(' '),
        }
    }
    ascii
}

/// Branch words the `smol` model (omp's role, as configured for `cwd`) picks for the task, for
/// [`worktree_branch`]; `None` when omp fails, times out or answers with no usable name.
pub(crate) async fn suggest(omp: &Path, extra_args: &[String], cwd: &str, title: Option<&str>, prompt: Option<&str>) -> Option<String> {
    let task = [title, prompt].into_iter().flatten().collect::<Vec<_>>().join("\n\n");
    let task: String = task.chars().take(SUGGEST_INPUT_CHARS).collect();
    if task.trim().is_empty() {
        return None;
    }
    let mut command = tokio::process::Command::new(omp);
    command
        .args(extra_args)
        .args(["-p", "--no-session", "--no-title", "--no-tools", "--no-skills", "--no-rules", "--no-extensions", "--no-lsp"])
        .args(["--allow-home", "--model", "@smol", "--thinking", "off", "--system-prompt", SYSTEM_PROMPT, "--cwd", cwd])
        // Starts with the instruction, so neither `@file` nor `-flag` parsing applies.
        .arg(format!("{INSTRUCTION}\n\n<task>\n{task}\n</task>"))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(SUGGEST_TIMEOUT, command.output()).await {
        Ok(Ok(output)) if output.status.success() => output,
        Ok(Ok(output)) => {
            tracing::warn!(status = %output.status, "branch name suggestion failed");
            return None;
        }
        Ok(Err(err)) => {
            tracing::warn!(%err, "cannot run omp for a branch name");
            return None;
        }
        Err(_) => {
            tracing::warn!("branch name suggestion timed out");
            return None;
        }
    };
    usable_answer(&String::from_utf8_lossy(&output.stdout))
}

/// The answer's first non-empty line when it reads as a name: 1 to [`SUGGEST_MAX_WORDS`] words.
fn usable_answer(answer: &str) -> Option<String> {
    let line = answer.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_matches(|c| c == '`' || c == '"' || c == '\'');
    let line = line.strip_prefix("coocoo/").unwrap_or(line);
    let words = ascii_words(line).split_whitespace().count();
    (1..=SUGGEST_MAX_WORDS).contains(&words).then(|| line.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_branches() {
        let id = "1a2b3c4d5e6f";
        assert_eq!(worktree_branch(Some("Git-Status: Push & PR"), id), "coocoo/git-status-push-pr-1a2b");
        assert_eq!(worktree_branch(Some("Lesbare Branch-Namen für Größe"), id), "coocoo/lesbare-branch-namen-fuer-groesse-1a2b");
        // Whole words up to the cap; a single overlong word is cut.
        let long = worktree_branch(Some("Worktree branches get readable names derived from the card title"), id);
        assert_eq!(long, "coocoo/worktree-branches-get-readable-names-1a2b");
        assert_eq!(worktree_branch(Some(&"x".repeat(100)), id), format!("coocoo/{}-1a2b", "x".repeat(BRANCH_TITLE_CHARS)));
        assert_eq!(worktree_branch(Some("🚀 …"), id), "coocoo/1a2b3c4d5e6f");
        assert_eq!(worktree_branch(None, id), "coocoo/1a2b3c4d5e6f");
    }

    #[test]
    fn model_answers() {
        assert_eq!(usable_answer("\n  `smarter-branch-names`\n").as_deref(), Some("smarter-branch-names"));
        assert_eq!(usable_answer("coocoo/fix-login-timeout").as_deref(), Some("fix-login-timeout"));
        // Chatter or nothing is no name; the title is used then.
        assert_eq!(usable_answer("Based on the task, I'd need the git state of a repository to work from"), None);
        assert_eq!(usable_answer("  \n"), None);
        assert_eq!(usable_answer("🚀"), None);
    }
}
