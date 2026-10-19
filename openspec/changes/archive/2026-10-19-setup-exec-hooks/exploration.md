# setup-exec-hooks — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 | S–M | — (50 local-isolation cubre el modo del directorio de datos; ver «Interacciones») | exploración cerrada; ver la [propuesta](proposal.md) |

**Tesis:** los hooks que instala `clawg setup claude-code` corren en cualquier plataforma y con cualquier shell que use el cliente, sin un shell entre el cliente y clawg, y `setup` no degrada los archivos que escribe (enlaces simbólicos, permisos, las copias que guarda). Hoy el hook es una cadena que el cliente pasa a `sh -c`, a Git Bash o a PowerShell: una ruta con espacio sin Git Bash no se puede ejecutar, un perfil de shell que imprime ensucia el JSON, y la falla es muda porque `clawg hook` siempre sale con 0. Y `write_file` reemplaza un `settings.json` enlazado por un archivo común y ensancha un 0600 a 0644.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El hook se escribe como una sola cadena `command`, sin `args` | **Cierta.** `hook_commands` arma la cadena (`crates/clawg-cli/src/setup.rs:287-293`) y `install_hooks` escribe `{type, command, timeout}` (`:449`). |
| `shell_word` normaliza `\` a `/` y deja sin comillas sólo `[A-Za-z0-9/._-:]` | **Cierta** (`:299-313`; `\` a `/` en `:300`, el conjunto en `:301`). Su comentario admite el límite: «the PowerShell a client falls back to does not run a quoted first word» (`:297-298`). |
| El diseño de client-setup eligió la forma de shell a propósito | **Cierta.** `openspec/changes/archive/2026-10-19-client-setup/design.md:41`: «Forma de shell, no `args`. La forma `args` (sin shell) es nueva en Claude Code; uno que no la conozca correría el binario solo, sin subcomando, y clap saldría con 2, que bloquea la sesión.» Y anota como riesgo aceptado que sin Git Bash, con espacios en la ruta, «esa combinación no funciona». Este cambio revierte esa decisión; la razón de hacerlo ahora está en «Problema». |
| `clawg` sin subcomando sale con 2 | **Cierta.** `command: Command` es obligatorio (`crates/clawg-cli/src/cli.rs:17-18`); `is_hook_invocation` sólo mira que el segundo argumento sea `hook` (`cli.rs:295`; su test afirma `!is_hook_invocation(["clawg"])`, `:654`), así que `main.rs:43-52` cae en `e.exit()`, que sale con 2. |
| El campo `args` existe en Claude Code y desactiva el shell | **Cierta.** Docs de hooks (https://code.claude.com/docs/en/hooks.md): «`args` … When present, `command` is resolved as an executable and spawned directly with `args` as the argument vector, with no shell involved». Y `shell`: «Ignored when `args` is set». En Windows: «exec form requires `command` to resolve to a real executable such as a `.exe`» (`clawg.exe` lo es). |
| La documentación dice desde qué versión existe `args` | **Falsa.** Las páginas no la dan. El CHANGELOG de anthropics/claude-code (https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md) sí: en 2.1.139, «Added hook `args: string[]` field (exec form) that spawns the command directly without a shell, so path placeholders never need quoting». |
| Sin Git Bash el cliente usa PowerShell | **Cierta**, textual en las docs: «The `command` string is passed to a shell: `sh -c` on macOS and Linux, Git Bash on Windows, or PowerShell when Git Bash isn't installed.» |
| Un perfil de shell que imprime ensucia el JSON | **Cierta, con matiz.** La guía (https://code.claude.com/docs/en/hooks-guide.md, «Shell profile output before the JSON») dice que los hooks corren en shells no interactivos, pero «Git Bash and some configurations, such as `BASH_ENV` pointing at `~/.bashrc`, still source your profile, and anything the profile prints reaches stdout ahead of your hook's JSON»; el cliente lee esa salida como texto y el JSON se ignora, y «Because the hook exited 0, the transcript shows no error either». Con `sh -c` común el caso es raro; con Git Bash o `BASH_ENV` no. |
| El shell suma al plazo de 3 s de `clawg hook` | **Cierta en el plazo, no medida en cuánto.** `HOOK_DEADLINE` = 3 s (`crates/clawg-cli/src/hook.rs:25`) contra `HOOK_TIMEOUT_SECS` = 5 (`setup.rs:40`). El arranque de un shell de login corre antes que clawg y se descuenta de esos 5 s. |
| `clawg hook` falla en silencio | **Cierta por diseño:** nada de su salida puede ser 2, y todo problema imprime nada y sale con 0 (`hook.rs:6-7`). El agente arranca sin contexto de memoria y nadie lo ve. |
| Un hook de clawg distinto del plan es un conflicto | **Cierta, y es el punto difícil.** `install_hooks` compara la cadena por igualdad (`setup.rs:439-440`) y lo reporta sin reemplazarlo; el spec lo fija en R3 (`openspec/specs/setup/spec.md:36`). Cambiar la forma del hook dejaría en conflicto a toda máquina que ya corrió `setup`. |
| `write_file` pierde enlaces y permisos | **Cierta.** La copia se hace con `std::fs::write` (`setup.rs:667-672`), el temporal `<archivo>.clawg-tmp` también (`:681-683`) y `std::fs::rename` lo pone en la ruta (`:684`). Un `settings.json` o `CLAUDE.md` enlazado (stow, chezmoi en modo enlace) pasa a ser archivo común; el temporal nace con el `umask`, así que un 0600 queda 0644; no hay `sync_all` antes del `rename`; no hay `read_link` ni `set_permissions` en `setup.rs`. Las copias nunca se podan. |

## Problema (verificado contra el código)

1. **Forma de shell.** El cliente entrega la cadena a un shell que clawg no controla. En Windows sin Git Bash la ruta por defecto, `%LOCALAPPDATA%\Programs\clawg` (`scripts/install.ps1:26`, `:37`), contiene el nombre de usuario: con un espacio o un carácter no ASCII (por ejemplo `C:\Users\Ana María\AppData\Local\Programs\clawg\clawg.exe`) `shell_word` la entrecomilla, y PowerShell no ejecuta una palabra entre comillas al principio. Resultado: error de parseo, el hook no corre, el agente arranca sin memoria y sin aviso. Las comillas que protegen en `sh` rompen en PowerShell, así que ninguna cadena sirve para los dos.
2. **Salida ajena.** Con Git Bash o `BASH_ENV`, lo que imprima el perfil precede al JSON y lo invalida (ver premisas).
3. **Plazo.** El arranque de un shell de login se descuenta de los 5 s del hook.
4. **Reconocimiento.** Para reconocer lo propio, `setup` lleva un analizador de shell: `split_command` (`setup.rs:324-367`) e `is_clawg_hook` (`:371-376`), sólo porque lo instalado es una cadena. Con `args` el hook es `command` + lista y no hace falta partir nada.
5. **Escritura.** `write_file` (`setup.rs:664-690`) degrada el destino (enlace, modo) y deja copias con el `umask` y sin límite.

## Lo que ya existe y se reutiliza

- `plan` puro sobre lo leído y `apply` que sólo ejecuta (`setup.rs:189`, `:627`): la migración se prueba sin disco.
- `is_clawg_program` (`:280-284`) reconoce `clawg` y `clawg.exe` por nombre de archivo y sirve para ambas formas.
- `install_hooks` y `remove_hooks` (`:421`, `:458`) y `hook_commands_of` (`:383-389`), que hoy sólo lee `command`.
- El servidor MCP ya es `command` + `args` y se compara por igualdad de ambos (`matches_server`, `:254-259`): es el modelo para el hook.
- Los tests existentes: `a_different_clawg_hook_is_a_conflict` (`:944`), `remove_after_setup_gives_back_the_files` (`:1043`), `apply_copies_the_previous_file_first` (`:1114`), `the_hook_program_is_quoted_only_when_it_needs_it` (`:1171`), y `tests/binary.rs` con un `claude` falso.
- El cliente con que se verificó setup fue Claude Code 2.1.303 (`openspec/specs/setup/spec.md:11`, `:64`), muy posterior a 2.1.139.

## Enfoques posibles

### Forma del hook

**A. Forma exec siempre, y reconocer la forma vieja como propia para migrarla (recomendado).** `command` = la ruta absoluta sin transformar (sin `\` a `/`, sin comillas) y `args` = `["hook", "<evento>", "--agent", "claude-code", "--profile", P]`. `is_clawg_hook` pasa a leer también `args`. La forma de shell que clawg generaba se reconoce como propia (con el analizador actual, reducido a lo necesario) y se **reescribe** en vez de reportarse como conflicto; `--remove` quita las dos formas.
- **A favor:** elimina las tres fallas (PowerShell, perfiles, shell de login); `shell_word` y la normalización desaparecen; una sola forma que probar.
- **En contra:** requiere que el cliente conozca `args` (2.1.139 o más); hay que decidir cuándo una cadena vieja «es de clawg» sin pisar una editada a mano.

**B. Exec sólo si `claude --version` es al menos 2.1.139, si no forma de shell.**
- **A favor:** no rompe clientes viejos.
- **En contra:** `setup` tendría que ejecutar `claude --version` (hoy sólo lo busca, `find_claude`, `:593`); el plan dejaría de depender sólo de lo leído; se conservan `shell_word` y `split_command`; y sin `claude` en el PATH hay que adivinar.

**C. Mantener la forma de shell; en Windows sin Git Bash escribir `& "<ruta>" hook …` con `"shell": "powershell"`.**
- **A favor:** no depende de `args`.
- **En contra:** `setup` tendría que detectar Git Bash; el hook queda atado a una máquina (el mismo `settings.json` sincronizado entre equipos deja de servir); no resuelve ni los perfiles ni el plazo.

**D. Que `clawg` sin subcomando salga con 1 y no con 2 (complemento de A; recomendado sólo si la decisión 1 pide proteger a clientes viejos).** Un cliente que ignora `args` correría `clawg` a secas; con 1 no bloquea la sesión, y desaparece la razón de `design.md:41` sin detectar versión.
- **A favor:** protege sin ejecutar nada desde `setup`.
- **En contra:** cambia el uso normal: hoy `clawg` sin argumentos imprime la ayuda y sale con 2 (`cli.rs:17-18`); habría que tratar ese caso en `main.rs` junto al manejo existente de errores de uso (`:43-52`) y fijarlo con un test. Y el riesgo sólo existe por debajo de 2.1.139.

**Combinación recomendada: A, con D si el usuario quiere cubrir clientes anteriores a 2.1.139.** A resuelve el problema; D es la red barata para el único caso que A deja abierto, sin meter ejecución de `claude` en `setup`.

### Escritura de archivos

**E. Escribir sobre el destino real, conservar el modo y no dejar copias abiertas (recomendado).** Resolver el destino con `std::fs::canonicalize` (o `read_link`) y crear el temporal junto al archivo real; en Unix copiar el modo original al temporal antes del `rename`; `sync_all` antes del `rename`; copias de `backups/setup` con modo 0600 en Unix (`OpenOptions` con `mode`); podar las copias viejas.
- **A favor:** sin dependencias, local a `write_file`.
- **En contra:** los tests de enlace y de modo son `#[cfg(unix)]` (en Windows un enlace pide privilegio o modo desarrollador); y la poda reabre R6 («MUST NOT tocar el directorio de datos de clawg salvo las copias de R7», `spec.md:54`), que hay que reconciliar.

**F. Escribir en el lugar, sin temporal ni `rename`.**
- **A favor:** conserva enlace y modo sin código extra.
- **En contra:** un corte deja el archivo a medias, y R7 exige temporal y `rename` (`spec.md:60`).

## Descartado

| Opción | Razón técnica |
|---|---|
| Un script envoltorio (`.cmd` o `.sh`) como `command` | Mete otro archivo que mantener y un shell otra vez; en Windows las docs dicen que un `.cmd` o `.bat` no se puede ejecutar en forma exec. |
| Escapar mejor la ruta para PowerShell dentro de la misma cadena | Lo que sirve a `sh` rompe en PowerShell (ver «Problema», punto 1). |
| Elegir la forma por el sistema operativo y no por el cliente | No dice qué shell tiene la máquina ni qué versión corre el cliente. |
| Escribir en el lugar (F) | Pierde la atomicidad que pide R7. |

## Interacciones

- **client-setup (archivado).** Este cambio revierte `design.md:41` y modifica su spec (`openspec/specs/setup/spec.md`): la definición de hook de clawg (`:14`) pasa a `command` + `args`, con la cadena vieja como forma heredada; R3 (`:36`) ya no pone la ruta «en forma de shell» y se va `shell_word`; el test `the_hook_program_is_quoted_only_when_it_needs_it` (`:38`) se reemplaza; R6 (`:54`) quita las dos formas y fija la poda; R7 (`:60`) suma destino y modo de la escritura; y `:64` («el hook corre por `sh -c`», Windows sin verificar) pasa a decir qué se verificó.
- **local-isolation (50).** Su exploración nombra las copias de `backups/setup` como nacidas con el `umask` (`openspec/changes/local-isolation/exploration.md:22`), lista `setup.rs:668` y `:679` entre los sitios que crean directorios (`:14`) y dice que su `umask` no cubre a `setup` por correr en otro proceso (`:53`). Reparto propuesto: 50 es dueño del directorio de datos y de su ayudante `ensure_private_dir` (`:52`) y `write_file` crea `<data>/backups/setup` con él cuando exista; este cambio conserva destino y modo de los archivos **del cliente** (`settings.json`, `CLAUDE.md`), el modo 0600 de cada copia y la poda. Si este cambio entra primero, crea el directorio con `create_dir_all` como hoy y 50 sustituye esa línea.
- **doctor-checks.** Un chequeo que lea los hooks instalados tendría que entender las dos formas; coordinar si ambos están abiertos.

## Alcance candidato

1. Hooks en forma exec (`command` + `args`), decididos por un `plan` puro.
2. Reconocer la forma de shell que clawg generaba y reescribirla; `--remove` quita ambas.
3. Retirar `shell_word` y la normalización; reducir `split_command` a lo que reconoce la forma vieja.
4. `clawg` sin subcomando sale con 1 (D), si la decisión 1 lo pide.
5. `write_file`: destino real tras enlaces, modo conservado, `sync_all`, copias 0600, poda.
6. Los cambios al spec de `setup` listados en «Interacciones».

### No-alcance

- Hooks de plugin del cliente o `${CLAUDE_PLUGIN_ROOT}`.
- Detectar la versión de `claude` desde `setup` (B).
- Mover el informe de `setup` a un módulo propio: `setup.rs` tiene 1182 líneas con las pruebas desde `:820`, y separarlo como `update` separa el suyo en `crates/clawg-cli/src/update/report.rs` es opcional y va en un commit aparte, igual que dejar de importar `crate::update::plan::stamp` sólo para la marca de las copias (`setup.rs:23`, `:670`).
- Otros clientes además de Claude Code.

## Decisiones

Cerradas con el usuario, todas con la recomendación.

- [x] **Piso de versión.** Forma exec desde 2.1.139, y D: `clawg` sin subcomando sale con 1, así un cliente anterior no bloquea la sesión.
- [x] **Qué es «forma vieja de clawg».** Sólo la cadena idéntica a la que generaba `hook_commands`, sea cual sea la ruta. Una editada a mano sigue en conflicto.
- [x] **Copias.** Se conservan las 5 más recientes por archivo; R6 se reconcilia en la spec.
- [x] **Enlace simbólico.** Se escribe sobre el destino real y el enlace queda.
- [x] **`clawg` sin argumentos.** Sale con 1 (D).

## Cómo se mide

Tests RED primero:

1. **Plan, hook nuevo.** `plan` sobre un estado vacío produce `command` = la ruta sin transformar y `args` = `hook <evento> --agent claude-code --profile P`, también con una ruta tipo `C:\Users\Ana María\...` (puro, en `setup.rs`). Hoy produce una cadena.
2. **Plan, migración.** Un `settings.json` con la cadena que generaba la versión anterior se planea como reescritura (no como conflicto) y una segunda corrida no escribe nada. Hoy es conflicto (`setup.rs:439`).
3. **Plan, ajeno.** Una cadena editada a mano que no cumple la regla de la decisión 2 sigue siendo conflicto o ajena.
4. **`--remove`.** Quita la forma exec y la vieja, y deja `settings.json` como antes.
5. **D**, si entra: `clawg` sin argumentos sale con 1 (`cli.rs`, `main.rs`); `clawg hook` mal formado sigue en 1.
6. **`write_file`, `#[cfg(unix)]`.** (a) Un `settings.json` que es enlace sigue siendo enlace y el destino cambia; (b) un 0600 queda 0600; (c) las copias nacen 0600; (d) tras N corridas quedan a lo sumo las copias elegidas. Hoy los cuatro fallan.
7. **Real.** Claude Code en una máquina Windows sin Git Bash y con un usuario cuya ruta tenga un espacio: el hook corre y el contexto llega. En Linux, con un perfil de shell que imprime a stdout: con la forma vieja el JSON se ignora, con la nueva no. Prueba manual documentada.

## Riesgos

- **Cliente anterior a 2.1.139:** ejecuta el binario sin argumentos. Con D no bloquea; sin D sale con 2 y bloquea la sesión.
- **Migración que pisa de más:** una cadena editada a mano parecida a la de clawg puede reescribirse. La decisión 2 lo acota.
- **`settings.json` sincronizado entre equipos:** la ruta absoluta del binario ya lo ataba a una máquina; la forma exec no lo empeora.
- **Poda de copias:** borra respaldos que un usuario pudo querer; el número es decisión suya y el reporte nombra las copias.
- **Enlaces en Windows:** no se pueden probar sin privilegio; los tests quedan en Unix.
