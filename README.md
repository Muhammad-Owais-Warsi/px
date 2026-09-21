# px

tiny windows helpers

## install

```powershell
irm https://raw.githubusercontent.com/Muhammad-Owais-Warsi/px/main/install.ps1 | iex
```

## usage

```
px rm <paths>        delete files/folders
px un <name>         uninstall app by name
px cp <file>         copy file content to clipboard
px apps              list installed apps with size
px ports             list active ports and services
px kill <port>       kill process on a port
px update            update to latest version
px destroy           uninstall px itself
```

## build from source

```powershell
cargo build --release
```
