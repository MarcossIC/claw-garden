# client-setup — Verificación

## Linux

Sobre `a985c12` (y otra vez después de los docs):

- `cargo test --workspace`: pasa entero.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sin advertencias.
- `serde_json` con `preserve_order` (D4) no movió ningún test, tampoco `the_agent_contract_matches_its_file`: el contrato se compara como JSON.

## Claude Code real (2.1.303), perfil dev, home temporal

Con `HOME`, `CLAUDE_CONFIG_DIR` y `CLAWG_TEST_HOME` en un directorio temporal, `CLAWG_TEST_EMBEDDER=fake` y el `claude` real en el PATH, sobre el build de tests de `target/debug/clawg`. Nunca se tocó `~/.claude`, `~/.claude.json` ni `~/.clawg*`.

1. Con un `CLAUDE.md` que decía `# Mine`, `clawg setup claude-code --profile dev` registró `clawg-dev`, agregó los dos hooks y el bloque, copió el `CLAUDE.md` previo a `.clawg-dev/backups/setup/` y salió con 0.
2. `claude mcp get clawg-dev`: `Scope: User config`, `Status: √ Connected`, `Command` la ruta absoluta del binario, `Args: mcp --profile dev --agent claude-code`. El chequeo de salud arrancó el daemon del perfil, como lo hace una sesión.
3. Una segunda corrida: `already registered`, los dos hooks `already there`, el protocolo `up to date`, sin copias nuevas y con 0.
4. El `command` de `SessionStart` que quedó en `settings.json`, corrido con `sh -c` como lo corre Claude Code y una entrada de `startup`, salió con 0 (sin memorias guardadas, no imprimió nada).
5. `--remove`: `removed` en los tres pasos; `settings.json` quedó en `{}`, `CLAUDE.md` volvió a `# Mine` y `claude mcp list` no lista ningún servidor. `clawg stop --profile dev` paró el daemon del paso 2.

## Lo que no se verificó

- **Windows.** La suite corre ahí en el CI del PR; los dos tests con un `claude` falso son sólo de Unix (un `claude.cmd` falso pasa por `cmd.exe`). Falta correr `clawg setup claude-code` con el `claude` de Windows, y en particular el `command` de los hooks en Git Bash con la ruta en `/`, y que una ruta con espacios no funciona sin Git Bash (D4).
- **Una sesión de Claude Code leyendo los hooks y el bloque.** El home temporal no tiene sesión iniciada; `clawg hook` ya se verificó con Claude Code en 25 con la configuración escrita a mano, que es la misma forma salvo la ruta absoluta y `timeout`.

## Veredicto

Sin CRITICAL. Los dos puntos de arriba quedan como WARNING para la primera instalación en Windows.
