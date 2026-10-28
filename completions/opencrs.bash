_opencrs() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="opencrs"
                ;;
            opencrs,check)
                cmd="opencrs__subcmd__check"
                ;;
            opencrs,completion)
                cmd="opencrs__subcmd__completion"
                ;;
            opencrs,conformance)
                cmd="opencrs__subcmd__conformance"
                ;;
            opencrs,dwell)
                cmd="opencrs__subcmd__dwell"
                ;;
            opencrs,fetch)
                cmd="opencrs__subcmd__fetch"
                ;;
            opencrs,legibility)
                cmd="opencrs__subcmd__legibility"
                ;;
            opencrs,probe)
                cmd="opencrs__subcmd__probe"
                ;;
            opencrs,publisher)
                cmd="opencrs__subcmd__publisher"
                ;;
            opencrs,resolve)
                cmd="opencrs__subcmd__resolve"
                ;;
            opencrs,rule-packs)
                cmd="opencrs__subcmd__rule__subcmd__packs"
                ;;
            opencrs,schema)
                cmd="opencrs__subcmd__schema"
                ;;
            opencrs,version)
                cmd="opencrs__subcmd__version"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        opencrs)
            opts="-h -V --help --version check schema resolve fetch publisher probe legibility dwell rule-packs conformance completion version"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__check)
            opts="-s -h --spec --files --no-files --json --now --margin --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -s)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --files)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --now)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --margin)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__completion)
            opts="-h --help bash zsh fish powershell"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__conformance)
            opts="-v -h --level --cmd --verbose --no-strictest --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --level)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --cmd)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__dwell)
            opts="-h --speed --near --far --ratio --rate --screen-width-mm --screen-width-px --pack --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --speed)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --near)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --far)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --ratio)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --rate)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --screen-width-mm)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --screen-width-px)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --pack)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__fetch)
            opts="-o -h --token --output --json --allow-local-http --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --output)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -o)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__legibility)
            opts="-h --near --far --ratio --screen-width-mm --screen-width-px --dwell --rate --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --near)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --far)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --ratio)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --screen-width-mm)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --screen-width-px)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --dwell)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --rate)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__probe)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__publisher)
            opts="-h --json --now --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --now)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__resolve)
            opts="-h --strictest --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__rule__subcmd__packs)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__schema)
            opts="-h --kind --allow-local-http --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --kind)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        opencrs__subcmd__version)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _opencrs -o nosort -o bashdefault -o default opencrs
else
    complete -F _opencrs -o bashdefault -o default opencrs
fi
