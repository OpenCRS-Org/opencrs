
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'opencrs' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'opencrs'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'opencrs' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Show the version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Show the version')
            [CompletionResult]::new('check', 'check', [CompletionResultType]::ParameterValue, 'Check a creative''s files against the screens they target')
            [CompletionResult]::new('schema', 'schema', [CompletionResultType]::ParameterValue, 'Check documents against the OpenCRS schemas')
            [CompletionResult]::new('resolve', 'resolve', [CompletionResultType]::ParameterValue, 'Show the requirements for a format or a screen')
            [CompletionResult]::new('fetch', 'fetch', [CompletionResultType]::ParameterValue, 'Find a publisher''s specs and check which to trust')
            [CompletionResult]::new('conformance', 'conformance', [CompletionResultType]::ParameterValue, 'Run the conformance suite against a validator')
            [CompletionResult]::new('completion', 'completion', [CompletionResultType]::ParameterValue, 'Print shell completions')
            [CompletionResult]::new('version', 'version', [CompletionResultType]::ParameterValue, 'Show the tool, spec and rule pack versions')
            break
        }
        'opencrs;check' {
            [CompletionResult]::new('-s', '-s', [CompletionResultType]::ParameterName, 'A screens.json to check against, repeatable')
            [CompletionResult]::new('--spec', '--spec', [CompletionResultType]::ParameterName, 'A screens.json to check against, repeatable')
            [CompletionResult]::new('--files', '--files', [CompletionResultType]::ParameterName, 'Directory holding the creative files')
            [CompletionResult]::new('--now', '--now', [CompletionResultType]::ParameterName, 'Validation time, RFC 3339 (default: now)')
            [CompletionResult]::new('--margin', '--margin', [CompletionResultType]::ParameterName, 'Warn below this multiple of the minimum text size')
            [CompletionResult]::new('--no-files', '--no-files', [CompletionResultType]::ParameterName, 'Use the manifest''s declared values, don''t read the files')
            [CompletionResult]::new('--json', '--json', [CompletionResultType]::ParameterName, 'Print the reports as a JSON array')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            break
        }
        'opencrs;schema' {
            [CompletionResult]::new('--kind', '--kind', [CompletionResultType]::ParameterName, 'Document kind (default: detect it)')
            [CompletionResult]::new('--allow-local-http', '--allow-local-http', [CompletionResultType]::ParameterName, 'Accept http://localhost document URLs in an index')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            break
        }
        'opencrs;resolve' {
            [CompletionResult]::new('--strictest', '--strictest', [CompletionResultType]::ParameterName, 'Combine every screen the reference covers into one set of requirements')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            break
        }
        'opencrs;fetch' {
            [CompletionResult]::new('--token', '--token', [CompletionResultType]::ParameterName, 'Bearer token for a publisher''s partner documents, repeatable')
            [CompletionResult]::new('-o', '-o', [CompletionResultType]::ParameterName, 'Save trusted documents under this directory')
            [CompletionResult]::new('--output', '--output', [CompletionResultType]::ParameterName, 'Save trusted documents under this directory')
            [CompletionResult]::new('--json', '--json', [CompletionResultType]::ParameterName, 'Print the result as JSON')
            [CompletionResult]::new('--allow-local-http', '--allow-local-http', [CompletionResultType]::ParameterName, 'Allow http:// and connections to localhost, for testing')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            break
        }
        'opencrs;conformance' {
            [CompletionResult]::new('--level', '--level', [CompletionResultType]::ParameterName, 'Level to test: core, layout or full')
            [CompletionResult]::new('--cmd', '--cmd', [CompletionResultType]::ParameterName, 'External validator to test')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'List passing cases too')
            [CompletionResult]::new('--verbose', '--verbose', [CompletionResultType]::ParameterName, 'List passing cases too')
            [CompletionResult]::new('--no-strictest', '--no-strictest', [CompletionResultType]::ParameterName, 'Skip the cases for optional strictest requirements')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            break
        }
        'opencrs;completion' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            break
        }
        'opencrs;version' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Show help (--help shows more)')
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
