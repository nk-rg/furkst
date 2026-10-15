<#
.SYNOPSIS
    Builds a set of demo repositories for trying out every part of Furkst by hand.

.DESCRIPTION
    Each repository is set up for one area: everyday work, conflicts, interactive rebase, merge styles, git-flow,
    Git LFS, and worktrees with submodules. Dates, authors and branches are realistic enough for the history graph,
    search and blame to have something to show. Run it again to start over; -Only rebuilds a single repository, for
    tests (like merging) that use their repository up.

.EXAMPLE
    .\scripts\make-demo-repos.ps1
    .\scripts\make-demo-repos.ps1 -Only merging
#>
param(
    # Where the repositories go. The script only ever deletes a folder it created itself.
    [string]$Root = (Join-Path $HOME "furkst-demos"),
    [ValidateSet("all", "basics", "conflicts", "rebase", "merging", "flow", "lfs", "extras")]
    [string]$Only = "all"
)

$ErrorActionPreference = "Stop"
$Marker = ".furkst-demos"
$Utf8 = New-Object Text.UTF8Encoding $false
# Non-ASCII names are built from code points, so the script reads the same in every PowerShell encoding.
$Jose = "Jos$([char]0xE9) N$([char]0xFA)$([char]0xF1)ez <jose@example.com>"
$Senal = "se$([char]0xF1)al"
$Authors = @("Ada Lovelace <ada@example.com>", "Grace Hopper <grace@example.com>", $Jose, "Linus Torvalds <linus@example.com>")

# Commit dates advance by about an hour and a half per commit, from the start of September 2026.
$script:Clock = [DateTimeOffset]::new(2026, 9, 1, 9, 0, 0, [TimeSpan]::FromHours(-5))

function Run([string]$Repo) {
    $ErrorActionPreference = "Continue"
    $out = & git.exe -C $Repo @args 2>&1
    if ($LASTEXITCODE -ne 0) { throw "git $($args -join ' ') failed in ${Repo}:`n$($out -join "`n")" }
    $out
}

# For commands expected to fail, like a merge that stops on conflicts.
function RunMayFail([string]$Repo) {
    $ErrorActionPreference = "Continue"
    & git.exe -C $Repo @args 2>&1 | Out-Null
}

function Put([string]$Repo, [string]$Path, [string]$Text) {
    $full = Join-Path $Repo $Path
    New-Item -ItemType Directory -Force (Split-Path $full) | Out-Null
    [IO.File]::WriteAllText($full, $Text.Replace("`r`n", "`n"), $Utf8)
}

function PutBytes([string]$Repo, [string]$Path, [int]$Seed, [int]$Size) {
    $bytes = New-Object byte[] $Size
    (New-Object Random $Seed).NextBytes($bytes)
    [IO.File]::WriteAllBytes((Join-Path $Repo $Path), $bytes)
}

function Png([string]$Repo, [string]$Path, [string]$Color) {
    Add-Type -AssemblyName System.Drawing
    $full = Join-Path $Repo $Path
    New-Item -ItemType Directory -Force (Split-Path $full) | Out-Null
    $bitmap = New-Object Drawing.Bitmap 96, 96
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $graphics.Clear([Drawing.Color]::FromName($Color))
    $graphics.FillEllipse([Drawing.Brushes]::White, 24, 24, 48, 48)
    $bitmap.Save($full, [Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
}

function Commit([string]$Repo, [string]$Message, [string]$Author = $Authors[0], [string]$Body = "") {
    $script:Clock = $script:Clock.AddMinutes(97)
    $env:GIT_AUTHOR_DATE = $env:GIT_COMMITTER_DATE = $script:Clock.ToString("yyyy-MM-ddTHH:mm:sszzz")
    try {
        Run $Repo add -A | Out-Null
        $messageArgs = @("-m", $Message) + $(if ($Body) { @("-m", $Body) } else { @() })
        Run $Repo commit -q --author $Author @messageArgs | Out-Null
    } finally {
        Remove-Item env:GIT_AUTHOR_DATE, env:GIT_COMMITTER_DATE
    }
}

# Removes the named folders under the root (left by an earlier run) and returns the first one's path.
function Fresh([string[]]$Names) {
    foreach ($name in $Names) {
        $dir = Join-Path $Root $name
        if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    }
    Join-Path $Root $Names[0]
}

function NewRepo([string]$Name, [switch]$Bare) {
    if ($Bare) { Run $Root init -q --bare -b main $Name | Out-Null } else { Run $Root init -q -b main $Name | Out-Null }
    $dir = Join-Path $Root $Name
    foreach ($setting in @(@("user.name", "Ada Lovelace"), @("user.email", "ada@example.com"), @("core.autocrlf", "false"))) {
        Run $dir config $setting[0] $setting[1] | Out-Null
    }
    $dir
}

function Lines([string]$Prefix, [int]$Count) {
    ((1..$Count) | ForEach-Object { "$Prefix $_" }) -join "`n"
}

# --- basics: status, staging, diffs, stashes, branches, tags, a remote, search, file history and blame -------------
function Basics {
    $repo = Fresh "basics", "basics-origin.git", "basics-teammate"
    $origin = NewRepo "basics-origin.git" -Bare
    $repo = NewRepo "basics"
    Put $repo ".gitmessage" "# Summary in 50 characters`n`n# Why this change?`n"
    Run $repo config commit.template .gitmessage | Out-Null

    Put $repo "README.md" "# Basics demo`n`nA small app to try Furkst on.`n"
    Put $repo "src/app.rs" ((1..40 | ForEach-Object { "fn step_$_() { println!(`"step $_`"); }" }) -join "`n")
    Commit $repo "Initial commit"
    Put $repo "src/parser.rs" "pub fn parse(input: &str) -> Vec<&str> {`n    input.split(',').collect()`n}`n"
    Commit $repo "Add a parser for comma-separated input" $Authors[1] "The parser splits on commas only; quoting comes later."
    Put $repo "docs/guide.md" "# Guide`n`n$(Lines 'Step' 12)`n"
    Commit $repo "Write the user guide" $Authors[2]
    Png $repo "assets/logo.png" "SteelBlue"
    PutBytes $repo "assets/data.bin" 1 4096
    Commit $repo "Add the logo and sample data"
    Put $repo "src/util.rs" "pub fn clamp(v: i32) -> i32 { v.max(0).min(100) }`n"
    Commit $repo "Add clamp helper" $Authors[3]
    Run $repo tag v0.1 | Out-Null
    Run $repo mv src/util.rs src/helpers.rs | Out-Null
    Commit $repo "Rename util to helpers" $Authors[1]
    Put $repo "src/parser.rs" "pub fn parse(input: &str) -> Vec<&str> {`n    input.split(',').map(str::trim).collect()`n}`n"
    Commit $repo "Fix parser keeping spaces around items" $Authors[2] "Reported by the $Senal team."
    Png $repo "assets/logo.png" "DarkOrange"
    Commit $repo "Brighter logo"
    for ($i = 1; $i -le 12; $i++) {
        Put $repo "config/settings.toml" "level = $i`nname = `"demo`"`n"
        Commit $repo "Tune settings (round $i)" $Authors[$i % 4]
    }
    Run $repo tag -a v1.0 -m "First stable release" | Out-Null

    # Branches in folders, and an old one merged back with a merge commit.
    foreach ($branch in "feature/login", "feature/search", "bugfix/crash-on-empty") {
        Run $repo checkout -q -b $branch main | Out-Null
        $file = "src/$($branch.Split('/')[1]).rs"
        Put $repo $file "// $branch`npub fn run() {}`n"
        Commit $repo "Start $branch" $Authors[2]
        Put $repo $file "// $branch`npub fn run() {`n    todo!()`n}`n"
        Commit $repo "Sketch $branch" $Authors[1]
    }
    Run $repo checkout -q -b old-experiment main | Out-Null
    Put $repo "src/experiment.rs" "pub fn try_it() {}`n"
    Commit $repo "Experiment with caching" $Authors[3]
    Run $repo checkout -q main | Out-Null
    $env:GIT_AUTHOR_DATE = $env:GIT_COMMITTER_DATE = $script:Clock.AddMinutes(30).ToString("yyyy-MM-ddTHH:mm:sszzz")
    Run $repo merge -q --no-ff old-experiment -m "Merge branch 'old-experiment'" | Out-Null
    Remove-Item env:GIT_AUTHOR_DATE, env:GIT_COMMITTER_DATE

    # The remote, a teammate pushing to it, then local work: main ends 2 ahead and 1 behind origin/main.
    Run $repo remote add origin $origin | Out-Null
    Run $repo push -q -u origin main feature/login v0.1 v1.0 | Out-Null
    Run $Root clone -q $origin basics-teammate | Out-Null
    $teammate = Join-Path $Root "basics-teammate"
    Run $teammate config user.name "Grace Hopper" | Out-Null
    Run $teammate config user.email "grace@example.com" | Out-Null
    Put $teammate "CHANGELOG.md" "# Changelog`n`n- Parser trims spaces`n"
    Commit $teammate "Start a changelog" $Authors[1]
    Run $teammate checkout -q -b feature/remote-only | Out-Null
    Put $teammate "src/remote.rs" "pub fn remote() {}`n"
    Commit $teammate "Work that only exists on the remote" $Authors[1]
    Run $teammate push -q origin main feature/remote-only | Out-Null
    Run $teammate checkout -q main | Out-Null
    Put $repo "src/app.rs" ((Get-Content (Join-Path $repo "src/app.rs")) -join "`n" -replace "step 3", "step three")
    Commit $repo "Spell out step three"
    Put $repo "README.md" "# Basics demo`n`nA small app to try Furkst on.`n`nRun it with ``cargo run``.`n"
    Commit $repo "Explain how to run it" $Authors[2]
    Run $repo fetch -q origin | Out-Null

    # Two stashes: one of tracked changes, one that also saved an untracked file.
    Put $repo "docs/guide.md" "# Guide (draft)`n`n$(Lines 'Step' 12)`n"
    Run $repo stash push -q -m "Guide title idea" | Out-Null
    Put $repo "src/parser.rs" "// TODO: quoting`n$((Get-Content (Join-Path $repo 'src/parser.rs')) -join "`n")`n"
    Put $repo "scratch/ideas.txt" "Quoted fields`nEscapes`n"
    Run $repo stash push -q -u -m "Parser quoting, with notes" | Out-Null

    # Work in progress: several hunks to stage by hunk or by line, a staged edit, untracked and deleted files, and
    # a whitespace-only change.
    $app = (Get-Content (Join-Path $repo "src/app.rs"))
    $app[1] = "fn step_2() { println!(`"step 2, now louder`"); }"
    $app[19] = "fn step_20() { println!(`"step 20`"); log(); }"
    $app[38] = "fn step_39() { println!(`"almost done`"); }"
    Put $repo "src/app.rs" (($app -join "`n") + "`nfn step_41() { println!(`"step 41`"); }")
    Put $repo "docs/guide.md" "# Guide`n`n$(Lines 'Step' 12)`n`nSee also the FAQ.`n"
    Run $repo add docs/guide.md | Out-Null
    Put $repo "notes.txt" "Things to remember`n"
    Remove-Item (Join-Path $repo "assets/data.bin")
    Put $repo "src/helpers.rs" "pub fn clamp(v: i32) -> i32 {  v.max(0).min(100)  }`n"
}

# --- conflicts: merge, rebase and cherry-pick that stop on conflicts ---------------------------------------------
function Conflicts {
    $repo = Fresh "conflicts"
    $repo = NewRepo "conflicts"
    Put $repo "story.txt" "Once upon a time`nthere was a repository`nwith two branches`nthat disagreed`nabout everything.`n"
    Put $repo "notes.md" "# Notes`n`nKeep this file.`n"
    Put $repo "config.toml" "color = `"blue`"`nsize = 3`n"
    Commit $repo "Write the story"
    Run $repo checkout -q -b topic | Out-Null
    Put $repo "story.txt" "Once upon a time`nthere was a TOPIC branch`nwith two branches`nthat agreed at last`nabout everything.`n"
    Commit $repo "Retell the story on topic" $Authors[1]
    Remove-Item (Join-Path $repo "notes.md")
    Put $repo "config.toml" "color = `"green`"`nsize = 3`n"
    Commit $repo "Drop notes, prefer green" $Authors[1]
    Run $repo checkout -q main | Out-Null
    Put $repo "story.txt" "Once upon a time`nthere was a MAIN branch`nwith two branches`nthat disagreed`nabout everything.`n"
    Put $repo "notes.md" "# Notes`n`nKeep this file. It matters.`n"
    Put $repo "config.toml" "color = `"red`"`nsize = 3`n"
    Commit $repo "Retell the story on main" $Authors[2]
}

# --- rebase: a messy linear history to clean up interactively ------------------------------------------------------
function Rebase {
    $repo = Fresh "rebase"
    $repo = NewRepo "rebase"
    $steps = @(
        @("recipe.md", "# Pancakes`n", "Add recipe skeleton"),
        @("recipe.md", "# Pancakes`n`n## Ingredients`n- flour`n- milk`n- egs`n", "Add ingredients"),
        @("recipe.md", "# Pancakes`n`n## Ingredients`n- flour`n- milk`n- eggs`n", "Fix typo in ingredients"),
        @("method.md", "## Method`n1. Mix`n2. Rest`n3. Fry`n", "Add the method"),
        @("method.md", "## Method`n1. Mix`n2. Rest 10 minutes`n3. Fry`n", "WIP"),
        @("serving.md", "Serve warm with syrup.`n", "Add serving notes"),
        @("debug.log", "temporary output`n", "Temporary debug output")
    )
    foreach ($step in $steps) {
        Put $repo $step[0] $step[1]
        Commit $repo $step[2] $Authors[$steps.IndexOf($step) % 3]
    }
    # Uncommitted, to show that a rebase sets it aside and puts it back.
    Put $repo "recipe.md" "# Pancakes (serves 4)`n`n## Ingredients`n- flour`n- milk`n- eggs`n"
}

# --- merging: one branch per merge style ---------------------------------------------------------------------------
function Merging {
    $repo = Fresh "merging"
    $repo = NewRepo "merging"
    Put $repo "app.txt" "version 1`n"
    Commit $repo "Start the app"
    Put $repo "app.txt" "version 2`n"
    Commit $repo "Version 2"
    foreach ($name in "diverged", "squash-me", "rebase-me") {
        Run $repo checkout -q -b "feature/$name" main | Out-Null
        foreach ($n in 1..3) {
            Put $repo "$name.txt" (Lines $name $n)
            Commit $repo "$name, part $n" $Authors[$n % 4]
        }
    }
    # Main moves on, so the branches above have diverged from it...
    Run $repo checkout -q main | Out-Null
    Put $repo "main.txt" "main moved on`n"
    Commit $repo "Main moves on" $Authors[3]
    # ...while this one is simply ahead of it: merging it can fast-forward.
    Run $repo checkout -q -b feature/ahead | Out-Null
    Put $repo "ahead.txt" "only ahead of main`n"
    Commit $repo "Work ahead of main" $Authors[1]
    Run $repo checkout -q main | Out-Null
}

# --- flow: git-flow initialized, with a feature in progress and a release to finish --------------------------------
function Flow {
    $repo = Fresh "flow"
    $repo = NewRepo "flow"
    Put $repo "README.md" "# Flow demo`n"
    Commit $repo "Initial commit"
    Run $repo tag 0.9 | Out-Null
    Run $repo branch develop | Out-Null
    foreach ($setting in @(@("gitflow.branch.master", "main"), @("gitflow.branch.develop", "develop"),
            @("gitflow.prefix.feature", "feature/"), @("gitflow.prefix.release", "release/"),
            @("gitflow.prefix.hotfix", "hotfix/"))) {
        Run $repo config $setting[0] $setting[1] | Out-Null
    }
    Run $repo checkout -q develop | Out-Null
    Put $repo "app.txt" "core`n"
    Commit $repo "Add the core" $Authors[1]
    Run $repo checkout -q -b feature/export | Out-Null
    Put $repo "export.txt" "export v1`n"
    Commit $repo "Add export" $Authors[2]
    Put $repo "export.txt" "export v2`n"
    Commit $repo "Polish export" $Authors[2]
    Run $repo checkout -q -b release/1.0 develop | Out-Null
    Put $repo "VERSION" "1.0`n"
    Commit $repo "Bump version to 1.0" $Authors[3]
    Run $repo checkout -q develop | Out-Null
}

# --- lfs: large files stored with Git LFS ---------------------------------------------------------------------------
function Lfs {
    $repo = Fresh "lfs"
    RunMayFail $Root lfs version
    if ($LASTEXITCODE -ne 0) { Write-Warning "Git LFS isn't installed; skipping the lfs demo."; return }
    $repo = NewRepo "lfs"
    Run $repo lfs install --local | Out-Null
    Run $repo lfs track "*.bin" | Out-Null
    PutBytes $repo "model.bin" 7 (512 * 1024)
    Put $repo "README.md" "# LFS demo`n`nmodel.bin is stored with Git LFS.`n"
    Commit $repo "Add the model"
    PutBytes $repo "model.bin" 8 (512 * 1024)
    Commit $repo "Retrain the model" $Authors[1]
    PutBytes $repo "model.bin" 9 (600 * 1024)
}

# --- extras: a submodule and a second worktree ----------------------------------------------------------------------
function Extras {
    $repo = Fresh "extras", "extras-hotfix", "shared-origin.git", "shared-work"
    $shared = NewRepo "shared-origin.git" -Bare
    Run $Root clone -q $shared shared-work 2>$null | Out-Null
    $work = Join-Path $Root "shared-work"
    Put $work "lib.txt" "shared library v1`n"
    Commit $work "Shared library v1" $Authors[3]
    Run $work push -q origin main | Out-Null
    $repo = NewRepo "extras"
    Put $repo "README.md" "# Extras demo`n"
    Commit $repo "Initial commit"
    Run $repo -c protocol.file.allow=always submodule add -q $shared libs/shared | Out-Null
    Commit $repo "Add the shared library as a submodule"
    Run $repo worktree add -q (Join-Path $Root "extras-hotfix") -b hotfix | Out-Null
    Remove-Item -Recurse -Force $work
}

if (Test-Path $Root) {
    if (-not (Test-Path (Join-Path $Root $Marker))) {
        throw "$Root already exists and wasn't made by this script. Choose another folder with -Root."
    }
} else {
    New-Item -ItemType Directory $Root | Out-Null
    New-Item -ItemType File (Join-Path $Root $Marker) | Out-Null
}

$builders = [ordered]@{ basics = "Basics"; conflicts = "Conflicts"; rebase = "Rebase"; merging = "Merging"; flow = "Flow"; lfs = "Lfs"; extras = "Extras" }
foreach ($name in $builders.Keys) {
    if ($Only -eq "all" -or $Only -eq $name) {
        Write-Host "Building $name..."
        & $builders[$name]
    }
}
Write-Host "Done: $Root"
