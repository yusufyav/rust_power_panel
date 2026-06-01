# bash completion for power_panel
_power_panel()
{
    local cur prev opts values

    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev=
    if (( COMP_CWORD > 0 )); then
        prev="${COMP_WORDS[COMP_CWORD-1]}"
    fi
    opts="--help -h --version -v --cli --tui --gui2 --debug --interval"
    values="0.5 1 2 5"

    if [[ $prev == "--interval" ]]; then
        COMPREPLY=( $(compgen -W "$values" -- "$cur") )
        return 0
    fi

    if [[ $cur == "--" ]]; then
        cur=
    fi

    COMPREPLY=( $(compgen -W "$opts" -- "$cur") )
    return 0
}

complete -F _power_panel power_panel
