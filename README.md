# px

tiny windows helpers

## install

```powershell
irm https://raw.githubusercontent.com/Muhammad-Owais-Warsi/px/main/install.ps1 | iex
```

## usage

```
px del <paths>       delete files/folders
px un <name>         uninstall app by name
px cp <file>         copy file content to clipboard
px apps              list installed apps with size
px destroy           uninstall px itself
```

## build from source

```powershell
cargo build --release
```
