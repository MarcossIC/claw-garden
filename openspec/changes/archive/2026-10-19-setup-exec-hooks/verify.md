# setup-exec-hooks — Verificación

## Linux

Sobre el commit de docs (`67487e2`):

- `cargo test --workspace`: pasa entero.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sin advertencias.
- Cada test nuevo de la [spec](spec.md) falló antes de su código: por no compilar (`is_bare_invocation`, `legacy_command`, `migrated`, `stale_copies`) o por la aserción (`a_bare_clawg_exits_1` salía con 2; el enlace pasaba a archivo común y el 0640 a 0644).

## `clawg setup` en Linux con el `claude` real (2.1.304), perfil dev, home temporal

Con `HOME`, `CLAUDE_CONFIG_DIR` y `CLAWG_TEST_HOME` en un directorio temporal, `CLAWG_TEST_EMBEDDER=fake` y el build de tests de `target/debug/clawg`. Nunca se tocó `~/.claude`, `~/.claude.json` ni `~/.clawg*`.

1. Un `settings.json` 0600 con los dos hooks como los escribía la versión anterior (la cadena `<ruta> hook <evento> --agent claude-code --profile dev`). `clawg setup claude-code --profile dev`: `migrated SessionStart, UserPromptSubmit to run without a shell`, sale con 0. Cada hook quedó con `command` = la ruta del binario y `args` = `hook <evento> --agent claude-code --profile dev`, en su grupo y con su matcher; `theme` siguió en su lugar. `settings.json` siguió en 0600 y su copia nació 0600.
2. `claude mcp get clawg-dev`: `Status: √ Connected`, la ruta absoluta y `--agent claude-code`.
3. Una segunda corrida: `already there` en los dos eventos, sin copias nuevas, sale con 0.
4. Con una memoria guardada y `BASH_ENV` apuntando a un archivo que imprime una línea: la forma heredada corrida por `bash -c` dio esa línea antes del JSON del hook; la forma exec, el binario con sus argumentos, dio sólo el JSON.
5. `clawg` sin argumentos salió con 1.
6. `--remove`: quitó el servidor, los dos hooks y el bloque; `settings.json` quedó con `theme` sólo. `clawg stop --profile dev` paró el daemon que arrancó el chequeo de `claude mcp get`.

## Windows

Sobre `develop` en `18c3c4f` (0.31.0), en Windows 11 con Git for Windows instalado:

- `cargo test --workspace --target-dir target-test`, después de `cargo clean --workspace --target-dir target-test`: pasa entero.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sin advertencias.

## `clawg setup` en Windows con el `claude` real (2.1.304), perfil dev, home temporal

El build de tests de `target-test\debug\clawg.exe` copiado a `<home>\Users\Ana María\AppData\Local\Programs\clawg\clawg.exe`, la ruta que deja el instalador para un usuario con un espacio y un carácter no ASCII en el nombre. `CLAUDE_CONFIG_DIR` y `CLAWG_TEST_HOME` en ese home y `CLAWG_TEST_EMBEDDER=fake`. El sha256 del `settings.json` de usuario y los `mcpServers` del `.claude.json` de usuario se compararon antes y después: no cambiaron.

1. Un `settings.json` con `theme`, un hook ajeno (`echo foreign`) en su propio grupo de `UserPromptSubmit` y los dos hooks como los escribía la versión anterior en Windows: la ruta con `/` y entre comillas dobles, seguida de `hook <evento> --agent claude-code --profile dev`. `clawg setup claude-code --profile dev`: registró `clawg-dev` con `claude mcp add`, `migrated SessionStart, UserPromptSubmit to run without a shell`, agregó el bloque y salió con 0. Cada hook quedó en su grupo con `command` = la ruta del binario con `\` y sin comillas y `args` = `hook <evento> --agent claude-code --profile dev`; el hook ajeno y `theme` siguieron en su lugar, y la copia del `settings.json` anterior quedó en `backups/setup`.
2. Una segunda corrida: `already registered`, `already there`, `up to date`, sin copias nuevas, sale con 0.
3. `claude mcp get clawg-dev`: `Status: ✔ Connected`, con la ruta con espacio y `--agent claude-code`. El chequeo dejó un daemon dev en el home temporal.
4. Una sesión real de Claude Code ejecutando los hooks exec. Una `user_preference` de `claude-code` con un marcador, guardada por `POST /v1/memories`; después, `claude -p` desde un directorio del home temporal con `--setting-sources project` (ni el `settings.json` de usuario ni sus plugins), `--settings` con el `settings.json` que escribió `setup` y `--strict-mcp-config`. El modelo contestó el marcador. El log de `--debug hooks` da los dos hooks de clawg con status 0 y el de `SessionStart:startup` con `provided additionalContext`, y muestra su línea de comando con la ruta sin comillas: un shell la habría partido en el espacio.
5. `clawg` sin argumentos salió con 1.
6. `--remove`: quitó el servidor, los dos hooks y el bloque (el `CLAUDE.md`, que sólo tenía el bloque, se borró); `settings.json` quedó con `theme` y el hook ajeno. `clawg stop --profile dev` paró el daemon.

## Lo que no se verificó

- **Windows sin Git Bash.** Claude Code 2.1.304 encuentra Git Bash en su ruta de instalación aunque no esté en el PATH, y con `CLAUDE_CODE_GIT_BASH_PATH` apuntando a un archivo que no existe avisa `falling back to auto-detection` y lo encuentra igual; la sesión de 4 se repitió de las dos maneras, con el mismo resultado. Con Git for Windows instalado no hay sesión que caiga a PowerShell. La forma exec no pasa por ningún shell (4); sin Git Bash cambia sólo la forma heredada, que `setup` ya no escribe.
- **Un archivo enlazado en Windows.** Los tests de enlace y de modo son de Unix.
- **Un cliente anterior a 2.1.139** corriendo `command` sin `args`: se verificó sólo que `clawg` a secas sale con 1, en Linux y en Windows.

## Veredicto

Sin CRITICAL. Windows y una sesión real ejecutando los hooks exec quedan verificados. Quedan como WARNING Windows sin Git Bash, un archivo enlazado en Windows y un cliente anterior a 2.1.139: ninguno se reproduce sin cambiar la máquina o instalar otro cliente, y ninguno cambia lo que escribe `setup`.
