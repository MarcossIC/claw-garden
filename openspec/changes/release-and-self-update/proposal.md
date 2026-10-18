# release-and-self-update — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | L | el arreglo de la poda de snapshots, como fix aparte y antes (ver «Requisitos previos»); se coordina con client-setup (26), doctor-checks (31) y daemon-hardening (38) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md); lo que sigue está verificado otra vez contra `main` (`65bb9d2`).

1. **Una versión nueva sólo se consigue compilando.** El README pide la toolchain, `cargo build --release -p clawg-cli` por `rustup run` (`README.md:52-63`), copiar el `.exe` junto al script y correrlo con `-ExecutionPolicy Bypass` (`README.md:107`, `:119`). En Linux no hay camino probado (`README.md:131`). Cada versión repite el ritual.
2. **No hay nada publicable.** `authors` nombra una persona y `license` y `repository` están vacíos (`Cargo.toml:7-9`); no hay `LICENSE` ni tags, y el único workflow es `ci.yml`.
3. **El binario no sabe de dónde vino ni si hay uno más nuevo.** Sólo distingue debug de release (`Profile::compiled_default`, `crates/clawg-core/src/paths.rs:60-66`): un `cargo build --release` local y un binario publicado son indistinguibles. La única comparación de versiones es por igualdad y sólo en `clawg memories` (`version_warning`, `crates/clawg-cli/src/memories.rs:332-338`).
4. **Reemplazar el binario deja a los editores sin memoria.** `install.ps1` para el daemon con el binario viejo y lo renombra a `.old-*` (`scripts/install.ps1:176-207`), pero no lo vuelve a levantar, y un puente abierto nunca arranca uno a mitad de sesión (`ClientHandle::reresolve`, `crates/clawg-cli/src/mcp/startup.rs:96-102`). Además, la pieza que arranca el daemon lanza `current_exe()` (`ProcessSpawner::spawn_serve`, `crates/clawg-cli/src/client.rs:943-946`), que durante una actualización es el binario viejo.
5. **El binario de release no es autocontenido.** ONNX Runtime va estático, pero en Windows importa el VC++ Redistributable, y en Linux enlaza `libstdc++`, glibc y, por las features por defecto de `reqwest` (`Cargo.toml:19`), `libssl` ([exploración](exploration.md#el-binario-del-release-es-autocontenido-pregunta-5)).

## Objetivo que sirve

Transversal: ningún objetivo llega al usuario si cada versión exige compilar. Y `update` va a correr las migraciones de todas las instalaciones, así que hereda la regla de los datos: deja al usuario con una versión que abre su base, o con la anterior y su daemon.

## Decisiones

Las doce abiertas de la exploración se resolvieron con la opción recomendada ([exploración](exploration.md#decisiones-abiertas)) y se suman a las siete que ya estaban tomadas.

**Publicación**

- **Licencia MIT** con el titular «The Claw Garden contributors», que pasa también a `authors`; `repository = "https://github.com/MarcossIC/claw-garden"`. Lo commiteado no nombra personas.
- **`release.yml` por `workflow_run` de `CI` en `main`.** Publica sólo con `event == 'push'`, `conclusion == 'success'` y `head_branch == 'main'`, y si la versión de `[workspace.package]` leída en el `head_sha` de esa ejecución no tiene tag `vX.Y.Z`; etiqueta ese `head_sha`. Sin la condición de `event`, la ejecución de CI de un PR desde un fork con una rama llamada `main` pasaría el filtro y el workflow firmaría código ajeno. El `chore: bump version to X.Y.Z` es el disparador.
- **`cargo build --release -p clawg-cli`, nunca `--workspace`:** clawg-eval enciende la feature `eval` de clawg-core (`crates/clawg-eval/Cargo.toml:34`), que «production builds never link» (`crates/clawg-core/Cargo.toml:16-19`), y la unificación de features la llevaría al binario. Con la marca `CLAWG_DIST_TARGET=<triple>` (`option_env!`), que un build local no tiene. Linux en `ubuntu-22.04` fijo, y el README declara su glibc mínima.
- **TLS: `rustls` con raíces del sistema en las dos plataformas**, en la raíz del workspace: `default-features = false` más `rustls-tls-native-roots`, `charset`, `http2` y `system-proxy` (las por defecto de `reqwest` 0.12.28 salvo `default-tls`). Saca `libssl` de las dependencias de runtime de Linux sin cambiar en qué CA se confía.
- **Assets sueltos, sin comprimir:** `clawg-x86_64-pc-windows-msvc.exe`, `clawg-x86_64-unknown-linux-gnu`, `install.ps1`, `SHA256SUMS` (de los tres) y `SHA256SUMS.minisig`, más la atestación de procedencia de GitHub. El release se crea cuando todos existen: `releases/latest` nunca ve uno a medias.
- **Firma minisign.** La clave privada y su contraseña son secretos de un environment `release` limitado a `main` por política de ramas, sin revisores obligatorios: los releases siguen siendo automáticos. El binario embebe una **lista** de claves públicas aceptadas desde el primer día, para poder rotar. La atestación queda para auditoría manual (`gh attestation verify`); `update` no la verifica.
- **Sin Authenticode.** SignPath Foundation queda como camino posterior.
- **El primer release es la versión que sube el archivo de este cambio.** El PR llega a `main` con el archivo y el bump en el mismo push: si `release.yml` aterrizara antes, publicaría con el número que hubiera en `main` un binario que no le corresponde. Hoy sería `v0.19.0`; si memory-provenance (41), en verify, se archiva antes, toma la `0.19.0` y este cambio publica la siguiente minor.

**Primera instalación (Windows)**

- **`install.ps1` va adjunto a cada release**, con la URL estable `https://github.com/MarcossIC/claw-garden/releases/latest/download/install.ps1` para `irm <url> | iex`. Baja el binario del último release, lo compara con `SHA256SUMS` y conserva lo que hace hoy: rechazar otra instalación, parar el daemon con el binario anterior, renombrar a `.old-*` y `clawg path add` (`scripts/install.ps1:146-223`). Con un `clawg.exe` local al lado sigue instalando ese, para desarrollo.
- **Si falta el VC++ Redistributable, lo dice** con el enlace oficial y no instala nada: ni el paquete ni DLL junto al `.exe`. `update` no lo necesita: el binario viejo ya arrancó.
- **Qué prueba esa verificación:** PowerShell 5.1 no verifica ed25519 sin una herramienta externa, y el script viene del mismo origen que el binario, así que el sha256 prueba la integridad de la descarga, no su autoría. Quien quiera más verifica con `minisign` antes de correrlo (README). La cadena de firma es la de `update`, con las claves embebidas en el binario ya instalado.

**`clawg update [--check]`**

- **Sólo el perfil `prod`.** Se niega en un build debug, sin la marca, o desde un binario dentro de un directorio de target de cargo, con un mensaje que dice por qué. La decisión es una función pura.
- **Sólo hacia una versión estrictamente mayor** (`X.Y.Z` de tres enteros, sin dependencia nueva): cierra el ataque de servir un release viejo con firma válida.
- **Sólo cuando se pide.** El daemon nunca consulta nada. Las peticiones mandan `User-Agent` (GitHub contesta 403 sin él).
- **Verifica antes de tocar nada:** la firma de `SHA256SUMS` con alguna clave de la lista (`minisign-verify`, la única dependencia nueva), el sha256 del binario bajado junto al destino y el `--version` del binario preparado.
- **El intercambio sigue el de `install.ps1`.** Si había daemon, lo para con el binario viejo (`stop_in`, `crates/clawg-cli/src/stop.rs:23`); renombra el actual a `.old-*` y pone el nuevo; y **sólo si había daemon** arranca el nuevo por su ruta explícita, no por `ProcessSpawner`, y espera `/health` con la versión nueva. Si no contesta, vuelven el binario viejo y su daemon.
- **Linux:** reemplaza el binario donde está. Sin permiso de escritura falla antes de bajar nada, con un mensaje claro; nunca `sudo`.
- **Informa** la versión anterior y la nueva, el snapshot `pre-migration` si se escribió, que volver atrás pierde lo escrito después (ver «Datos y vuelta atrás»), y que los editores abiertos siguen con el puente viejo hasta reiniciarse.

## Alcance

Un solo cambio de openspec con tres tandas. Cada una deja algo útil y se commitea por separado **en la misma rama**; ninguna llega sola a `main`, porque la primera publicaría la versión que hubiera ahí.

1. **Pipeline e instalación.** `LICENSE` y metadatos (`Cargo.toml:7-9`); `reqwest` con `rustls` (`Cargo.toml:19`); la marca de compilación; `.github/workflows/release.yml` (acciones fijadas por SHA, humo `--version` contra el tag, `ldd` sin `libssl` en Linux); la clave pública en el repositorio; `scripts/install.ps1` como bootstrap; README (instalar con una línea, verificar lo bajado, Linux y su glibc mínima, SmartScreen y Smart App Control); `.claude/CLAUDE.md` (Versioning y CI).
2. **`clawg update --check`.** Los módulos puros: versión, nombre del asset por triple, elección del release en el JSON de la API (sin borradores ni prereleases), `SHA256SUMS`, firma contra la lista de claves y la decisión de si este binario puede actualizarse. La consulta HTTP. La URL base y las claves son sustituibles sólo en un build `testing`, con la regla de `CLAWG_TEST_HOME`. Códigos de salida que distinguen «hay una nueva», «al día» y «error».
3. **`clawg update`.** El plan de pasos como datos y su ejecución: descarga junto al destino, verificación, humo, parada, intercambio, arranque por ruta, confirmación por `/health` y vuelta atrás; los `.old-*` de corridas anteriores se borran al empezar. README («Actualizar» y la vuelta atrás) y la lista Commands de `.claude/CLAUDE.md`.

Specs nuevas: `release` (workflow, assets, firma, bootstrap) y `self-update` (el comando); no `update`, que es la de `update_memory`. `specs/install` se modifica en su sección CI, que hoy dice que corre en cada push y cita líneas de `ci.yml` anteriores a `paths-ignore` (`.github/workflows/ci.yml:8-26`).

## Fuera de alcance

- Authenticode, macOS, Windows ARM, `winget` o Scoop.
- Un actualizador en segundo plano, una consulta automática o un canal beta.
- Un instalador para Linux: se publica el binario y dónde va es decisión del usuario.
- Instalar el VC++ Redistributable o distribuir sus DLL.
- Verificar la atestación dentro de `update`.
- Parar o relanzar un daemon `dev` servido por el binario instalado.
- Cambios de esquema, identificadores, estados o contrato de embedding.
- El arreglo de la poda de snapshots: es un requisito previo.

## Requisitos previos

- **La poda conserva el `pre-migration`.** `to_prune` elegía los automáticos fuera de los 5 más nuevos sin mirar el motivo, así que una racha de restores podía sacar la vuelta atrás de una actualización. El arreglo, conservar el más nuevo de cada motivo (`crates/clawg-core/src/memory/backup.rs:233-238`), entra antes como fix aparte con su bump de patch.
- **El par de claves minisign**, generado fuera del repositorio por quien lo administra: la privada y su contraseña en el environment `release`, la pública commiteada y embebida.
- **La API de `minisign-verify`**, verificada y con la versión fijada en el diseño: la exploración la da como publicada hace pocas semanas.

## Cómo se mide el éxito

Tests que fallan hoy (no compilan). Ninguno toca `~/.clawg*` ni la red real.

1. Versión: `0.18.0 < 0.19.0`, iguales, más vieja; `v0.19`, `0.19.0-rc1` y vacío se rechazan.
2. Elegibilidad: un build debug, sin marca, bajo un target de cargo o en `dev` se niega con su motivo; un release marcado en `prod` pasa.
3. Asset por triple; la elección salta borradores y prereleases; sin el asset propio, el error nombra el triple.
4. `SHA256SUMS`: línea faltante, hash corto, nombre duplicado.
5. Firma: válida; con la segunda clave de la lista; con una clave fuera de la lista; con el archivo alterado; con el `.minisig` de otro archivo.
6. Plan: con y sin daemon; cada falla deshace lo que corresponde; un hash que no coincide no toca el binario instalado.
7. Por el binario real, con un servidor de release falso y claves de prueba: actualización completa con daemon (contesta la versión nueva, el viejo queda como `.old-*` y un puente abierto llega al daemon nuevo: el escenario de `crates/clawg-cli/tests/binary.rs:535` con el binario reemplazado); un binario nuevo que no arranca (vuelven el viejo y su daemon); `--check` con y sin versión nueva.
8. Base sembrada en la versión anterior con una migración pendiente: aparece el `pre-migration` y los datos siguen alcanzables. Un store de un clawg más nuevo: el daemon nuevo se niega y el usuario queda con el binario anterior y su daemon.
9. Convención: la lista de claves embebida no contiene la clave de prueba.

En el workflow, fuera de la suite: el binario responde `--version` con la versión del tag, el de Linux no enlaza `libssl`, `minisign -Vm SHA256SUMS` pasa con la clave del repositorio y el release aparece con sus cinco archivos y la atestación. A mano, en Windows: `irm … | iex` instala y `clawg update --check` dice «al día».

`rag-eval` no aplica: no cambia el ranking.

## Datos y vuelta atrás

No cambia el esquema, ningún identificador ni estado, ni el contrato de embedding; pero `update` va a correr las migraciones de todas las instalaciones.

- **Antes del intercambio** no se toca nada: una falla de descarga, firma, hash o humo deja todo como estaba.
- **Después**, si el daemon nuevo no contesta, vuelven el binario viejo y su daemon. Si alcanzó a migrar, el viejo se niega (`MemoryError::SchemaNewer`, `crates/clawg-core/src/memory/error.rs:54-58`): el diseño decide entre restaurar el `pre-migration` en el acto (`restore` guarda antes el store reemplazado, así que nada se destruye) o dejar el binario nuevo y decir cómo volver. Lo que no puede pasar es que el usuario quede sin daemon y sin saber por qué.
- **A mano, tras una actualización que anduvo:** `clawg stop`, devolver el `.old-*` a su nombre y `clawg restore` del `pre-migration` (el que se escribe al abrir un store con migraciones pendientes, `crates/clawg-core/src/memory/db.rs:207-213`). Lo escrito después no se borra: queda en el `pre-restore`, que sólo la versión nueva abre. Sin migración alcanza con el binario.
- **Revertir el cambio:** sin `release.yml` no se publica más; los binarios instalados siguen andando y `update` dice «al día». Volver a OpenSSL es devolverle a `reqwest` sus features por defecto: las raíces de confianza son las mismas.
- **Clave comprometida:** un release firmado con la clave siguiente cuya lista ya no incluye la filtrada; quien no actualice la sigue aceptando, y el README lo dice. La versión estrictamente mayor limita el daño a releases nuevos.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| `workflow_run` corre con secretos y con el workflow de `main` aunque lo dispare la CI de un PR | la condición `event == 'push'`. La política de ramas del environment no alcanza: un job de `workflow_run` siempre corre en la rama por defecto, así que sólo frena workflows de otras ramas. El diseño evalúa comparar también `head_repository` |
| la primera ejecución real de `release.yml` es la del primer release (sólo corre desde `main`) | publicar al final: una falla no deja nada visible, y la próxima CI exitosa en `main` reintenta mientras la versión no tenga tag |
| una migración o un re-embed largos demoran el primer `/health` del daemon nuevo | el plazo distingue «arrancando» de «no arrancó» (diseño) |
| un editor abre un puente entre la parada y el arranque, y su autoarranque lanza otro daemon | `update` confirma la versión del daemon que contesta y lo informa |
| dos `update` a la vez | un cierre junto al binario o un error claro (diseño) |
| VC++ y DirectML (`d3d12`, `directml`, `dxgi`, importaciones normales) en un Windows limpio | sin verificar: se prueba el bootstrap en una máquina limpia antes de prometerlo |
| antivirus, SmartScreen (la reputación se reinicia con cada hash) y Smart App Control | ninguna sin Authenticode; el README lo dice |
| 60 consultas por hora sin autenticar a la API de GitHub | la consulta sólo corre a pedido |
