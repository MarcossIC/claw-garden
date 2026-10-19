# Settings of the PSScriptAnalyzer run in .github/workflows/installer.yml. Every
# other rule applies, and any finding fails the run.
@{
    ExcludeRules = @(
        # The installer talks to the person running it, and piped to
        # Invoke-Expression its Write-Output would land in the caller's pipeline.
        'PSAvoidUsingWriteHost',
        # The parameters are read inside the `& { }` block that keeps the
        # installer out of the caller's scope, which the rule does not follow.
        'PSReviewUnusedParameter',
        # The helpers are private to the script, not cmdlets: -WhatIf would
        # promise a dry run the installer does not have.
        'PSUseShouldProcessForStateChangingFunctions'
    )
}
