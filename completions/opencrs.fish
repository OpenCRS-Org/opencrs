# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_opencrs_global_optspecs
    string join \n h/help V/version
end

function __fish_opencrs_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_opencrs_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_opencrs_using_subcommand
    set -l cmd (__fish_opencrs_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c opencrs -n "__fish_opencrs_needs_command" -s h -l help -d 'Show help (--help shows more)'
complete -c opencrs -n "__fish_opencrs_needs_command" -s V -l version -d 'Show the version'
complete -c opencrs -n "__fish_opencrs_needs_command" -f -a "check" -d 'Check a creative\'s files against the screens they target'
complete -c opencrs -n "__fish_opencrs_needs_command" -f -a "schema" -d 'Check documents against the OpenCRS schemas'
complete -c opencrs -n "__fish_opencrs_needs_command" -f -a "resolve" -d 'Show the requirements for a format or a screen'
complete -c opencrs -n "__fish_opencrs_needs_command" -f -a "fetch" -d 'Find a publisher\'s specs and check which to trust'
complete -c opencrs -n "__fish_opencrs_needs_command" -f -a "conformance" -d 'Run the conformance suite against a validator'
complete -c opencrs -n "__fish_opencrs_needs_command" -f -a "completion" -d 'Print shell completions'
complete -c opencrs -n "__fish_opencrs_needs_command" -f -a "version" -d 'Show the tool, spec and rule pack versions'
complete -c opencrs -n "__fish_opencrs_using_subcommand check" -s s -l spec -d 'A screens.json to check against, repeatable' -r
complete -c opencrs -n "__fish_opencrs_using_subcommand check" -l files -d 'Directory holding the creative files' -r
complete -c opencrs -n "__fish_opencrs_using_subcommand check" -l now -d 'Validation time, RFC 3339 (default: now)' -r
complete -c opencrs -n "__fish_opencrs_using_subcommand check" -l margin -d 'Warn below this multiple of the minimum text size' -r
complete -c opencrs -n "__fish_opencrs_using_subcommand check" -l no-files -d 'Use the manifest\'s declared values, don\'t read the files'
complete -c opencrs -n "__fish_opencrs_using_subcommand check" -l json -d 'Print the reports as a JSON array'
complete -c opencrs -n "__fish_opencrs_using_subcommand check" -s h -l help -d 'Show help (--help shows more)'
complete -c opencrs -n "__fish_opencrs_using_subcommand schema" -l kind -d 'Document kind (default: detect it)' -r
complete -c opencrs -n "__fish_opencrs_using_subcommand schema" -l allow-local-http -d 'Accept http://localhost document URLs in an index'
complete -c opencrs -n "__fish_opencrs_using_subcommand schema" -s h -l help -d 'Show help (--help shows more)'
complete -c opencrs -n "__fish_opencrs_using_subcommand resolve" -s h -l help -d 'Show help (--help shows more)'
complete -c opencrs -n "__fish_opencrs_using_subcommand fetch" -l token -d 'Bearer token for a publisher\'s partner documents, repeatable' -r
complete -c opencrs -n "__fish_opencrs_using_subcommand fetch" -s o -l output -d 'Save trusted documents under this directory' -r
complete -c opencrs -n "__fish_opencrs_using_subcommand fetch" -l json -d 'Print the result as JSON'
complete -c opencrs -n "__fish_opencrs_using_subcommand fetch" -l allow-local-http -d 'Allow http:// and connections to localhost, for testing'
complete -c opencrs -n "__fish_opencrs_using_subcommand fetch" -s h -l help -d 'Show help (--help shows more)'
complete -c opencrs -n "__fish_opencrs_using_subcommand conformance" -l level -d 'Level to test: core, layout or full' -r
complete -c opencrs -n "__fish_opencrs_using_subcommand conformance" -l cmd -d 'External validator to test' -r
complete -c opencrs -n "__fish_opencrs_using_subcommand conformance" -s v -l verbose -d 'List passing cases too'
complete -c opencrs -n "__fish_opencrs_using_subcommand conformance" -s h -l help -d 'Show help (--help shows more)'
complete -c opencrs -n "__fish_opencrs_using_subcommand completion" -s h -l help -d 'Show help (--help shows more)'
complete -c opencrs -n "__fish_opencrs_using_subcommand version" -s h -l help -d 'Show help (--help shows more)'
