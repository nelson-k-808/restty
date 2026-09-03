use crate::config::Config;

pub fn init(shell: &str, config: &Config) -> Option<String> {
    match shell {
        "bash" => Some(generate(shell, config, "${PIPESTATUS[0]}")),
        "zsh" => Some(generate(shell, config, "${pipestatus[1]}")),
        _ => None,
    }
}

fn generate(shell: &str, config: &Config, pipeline_status: &str) -> String {
    let mut output = format!(
        r#"# restty shell integration for {shell}
__restty_should_render() {{
    [ -t 1 ] && [ "${{RESTTY_DISABLE:-0}}" != "1" ] && [ -z "${{NO_COLOR+x}}" ] && command -v restty >/dev/null 2>&1
}}

__restty_run() {{
    local restty_command_name="$1"
    shift
    if [ "${{RESTTY_LIVE:-0}}" = "1" ]; then
        restty live --source="$restty_command_name" -- "$restty_command_name" "$@"
        return $?
    fi
    command "$restty_command_name" "$@" | restty render --source="$restty_command_name"
    local restty_command_status={pipeline_status}
    return "$restty_command_status"
}}

__restty_ls_is_long() {{
    local restty_argument
    for restty_argument in "$@"; do
        case "$restty_argument" in
            --) break ;;
            --format=long|--format=verbose) return 0 ;;
            -*) case "$restty_argument" in *l*) return 0 ;; esac ;;
        esac
    done
    return 1
}}

__restty_git_is_supported() {{
    local restty_git_mode="${{1:-}}"
    local restty_argument
    shift 2>/dev/null || return 1
    for restty_argument in "$@"; do
        case "$restty_git_mode:$restty_argument" in
            status:--short|status:-s|status:--porcelain|status:--porcelain=*) return 0 ;;
            log:--oneline|log:--pretty=oneline) return 0 ;;
        esac
    done
    return 1
}}

"#,
    );

    for command in &config.enabled_commands {
        match command.as_str() {
            "git" => output.push_str(
                r#"git() {
    if __restty_git_is_supported "$@" && __restty_should_render; then __restty_run git "$@"; else command git "$@"; fi
}

"#,
            ),
            "docker" => output.push_str(
                r#"docker() {
    case "${1:-}" in
        ps) if __restty_should_render; then __restty_run docker "$@"; else command docker "$@"; fi ;;
        *) command docker "$@" ;;
    esac
}

"#,
            ),
            "ls" => output.push_str(
                r#"ls() {
    if __restty_ls_is_long "$@" && __restty_should_render; then __restty_run ls "$@"; else command ls "$@"; fi
}

"#,
            ),
            "ps" => output.push_str(
                r#"ps() {
    case "${1:-}" in
        aux|-aux) if __restty_should_render; then __restty_run ps "$@"; else command ps "$@"; fi ;;
        *) command ps "$@" ;;
    esac
}

"#,
            ),
            "ip" => output.push_str(
                r#"ip() {
    case "${1:-}:${2:-}" in
        route:*|r:*|-4:route|-4:r|-6:route|-6:r) if __restty_should_render; then __restty_run ip "$@"; else command ip "$@"; fi ;;
        *) command ip "$@" ;;
    esac
}

"#,
            ),
            "df" | "du" | "lsblk" | "free" => {
                output.push_str(&format!(
                    r#"{command}() {{
    if __restty_should_render; then __restty_run {command} "$@"; else command {command} "$@"; fi
}}

"#
                ));
            }
            _ if valid_shell_name(command) => {
                output.push_str(&format!(
                    r#"{command}() {{
    if __restty_should_render; then __restty_run {command} "$@"; else command {command} "$@"; fi
}}

"#
                ));
            }
            _ => {}
        }
    }
    output
}

fn valid_shell_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_uses_shell_specific_pipeline_status() {
        let config = Config::default();
        let bash = init("bash", &config).unwrap();
        assert!(bash.contains("${PIPESTATUS[0]}"));
        assert!(bash.contains("__restty_ls_is_long"));
        assert!(bash.contains("${RESTTY_LIVE:-0}"));
        assert!(init("zsh", &config).unwrap().contains("${pipestatus[1]}"));
    }
}
