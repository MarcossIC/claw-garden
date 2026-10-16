# project-root-guard — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2, 3 | S | — (project-identity y project-indexing ya están archivados) | exploración (borrador) |

**Tesis:** hoy el modelo decide qué directorio del disco indexa el daemon, y una colisión de `project_id` mezcla dos proyectos sin avisar. Son dos huecos distintos de la misma puerta: `project_path`. Ninguno pierde datos hoy, pero los dos son silenciosos. Este cambio no toca el esquema ni el id: cierra la puerta (qué raíces se registran) y hace ruidoso el choque (qué pasa cuando dos raíces dan el mismo id).

## Premisas verificadas

| Premisa | Veredicto |
|---|---|
| `search_project` registra e indexa cualquier `project_path` | **Cierto, con un matiz.** Ver abajo: el camino pasa por el registro, que también usan las otras cuatro herramientas. |
| No hay lista de raíces permitidas | **Cierto.** `register` acepta cualquier `root_path` no vacío (`crates/clawg-cli/src/daemon/api.rs:1021-1029`). |
| Respeta `.gitignore` y enmascara secretos | **Cierto, pero el enmascarado es parcial.** `redact` sólo cubre formatos conocidos (`crates/clawg-core/src/memory/project_docs/mod.rs:378`, `crates/clawg-core/src/memory/redact.rs`). Un `.env.md` con una contraseña común pasa. |
| `project_id` es un hash de 16 bits y dos raíces pueden chocar a 1 en 65.536 por par | **Parcialmente cierto.** El id es `{basename}-{hash4}` (`crates/clawg-core/src/memory/identity.rs:11-15`, `:152`). Sólo chocan raíces con el **mismo nombre de carpeta en minúsculas**. `app-1a2b` nunca choca con `api-1a2b`. El 1 en 65.536 vale por par con igual nombre (dos `app`, dos `web`, dos `frontend`), no por par cualquiera. |
| En un 409 el puente sólo loguea y sigue con el mismo id | **Cierto.** `crates/clawg-cli/src/mcp/server.rs:518-520`. Es deliberado: la regla R6 de `specs/project` dice que un registro fallido no bloquea un save, y el test `a_refused_registration_does_not_block_the_save` lo fija (`server.rs:1176-1199`). |
| Hay un test que cubra la mezcla silenciosa | **No.** Ese test comprueba que el save **no se bloquea** y que cae 1 fila bajo el id (`server.rs:1195-1198`); no comprueba que el agente se entere. El 409 del daemon sí está probado (`crates/clawg-core/src/memory/project.rs:286-309`, HTTP en `crates/clawg-cli/tests/api.rs`). |

## Problema (verificado contra el código)

### 1. Qué raíces se registran

- Cinco herramientas toman `project_path` y todas pasan por `project_for`: `save_memory` (`server.rs:541`), `search_memory` (`:570`), `recall_context` (`:596`), `search_project` (`:722`) y `synthesize` (`:747`). Sus campos son `server.rs:141`, `:159`, `:174`, `:188`, `:199`.
- `resolve_project` da prioridad a `project_path` sobre el cwd del puente (`crates/clawg-cli/src/mcp/tools.rs:301-314`). `ProjectIdentity::resolve` sube hasta el primer `.git` o, fuera de git, deja el path canónico (`identity.rs:41-53`). Un `project_path` de `C:\Users\dev` sin `.git` arriba es la raíz **entera**.
- `project_for` registra la raíz una vez por proceso (`server.rs:515-521`). El daemon guarda `(id, root_path)` y lanza el refresco del índice en segundo plano (`api.rs:1036`, `spawn_doc_refresh` en `api.rs:1044-1064`).
- Una ruta inexistente también se registra (se normaliza a texto, `identity.rs:44-47`), pero el índice falla con `NotADirectory` y sólo loguea (`crates/clawg-core/src/memory/project_docs/corpus.rs:70-72`, `api.rs:1061`). Queda una fila basura en `project`.
- Lo que el índice lee: Markdown, manifiestos y código Rust, Python y TypeScript, hasta 2.000 archivos de 256 KB (`corpus.rs:14-17`), sin seguir enlaces simbólicos (`corpus.rs:81`) y saltando lo ignorado (`corpus.rs:73-83`). Se guarda en `project_doc_*` y viaja en los backups.
- `search_project` devuelve esos pasajes. `POST /v1/projects/docs/search` toma sólo `project_id`, **sin agente** (`api.rs:1087-1092`) y `sharing.json` no menciona proyectos (`crates/clawg-core/src/memory/sharing.rs`, cero coincidencias de «project»). Un agente indexa; cualquier otro lee por el mismo path.

**Qué está en juego de verdad.** Para las herramientas de memoria, `project_path` es sólo una etiqueta de partición: `all_projects` ya alcanza todo (`search.rs:179`, `mcp/server.rs:163`). El riesgo concreto es el **índice de documentos**: es el único camino por el que el daemon lee el disco por pedido del modelo. El cliente (Claude Code) tiene su propio modelo de permisos para leer archivos; este camino lo esquiva. Con un agente que ya tiene shell o `Read` sin restricción, el daemon no suma capacidad. Con uno restringido (o con una inyección de prompt en un repo ajeno), sí.

### 2. Qué pasa en una colisión

- `register_project` hace `INSERT OR IGNORE` y compara la `root_path` guardada; si difiere, devuelve `RootCollision` sin mover filas (`project.rs:79-87`, `:113`). El daemon contesta 409 (`api.rs:1033`).
- El puente descarta ese error con un `log::warn!` y devuelve igual `project.id` (`server.rs:518-522`). Como `registered` ya contiene la clave, **no vuelve a intentarlo ni a avisar** en el resto del proceso (`server.rs:515`).
- Consecuencias en cadena, todas silenciosas:
  - Los saves de la segunda raíz caen en el mismo `project_id` que los de la primera. La búsqueda no distingue.
  - `search_project` de la segunda raíz sirve los documentos de la **primera**: `root_of` devuelve la `root_path` guardada (`project_docs/mod.rs:426-435`) y el refresco indexa esa carpeta (`mod.rs:221`).
  - Las filas ya mezcladas **no se pueden separar después**: ninguna fila guarda la raíz de la que salió. Ensanchar el hash más tarde no las arregla (ver 30).
- El aviso sólo está en el log del puente (`stderr` del cliente), que casi nadie lee.

## Lo que ya existe y se reutiliza

- `RegisterError::RootCollision { id, existing }` y el 409 con su mensaje (`project.rs:30-33`, `api.rs:1033`). El daemon ya detecta el choque y ya dice contra qué raíz.
- La tabla `project (id, root_path, created_at)` (`crates/clawg-core/migrations/V9__project.sql:6-10`): es la lista de raíces conocidas. No hace falta tabla nueva para saber qué está registrado.
- `ProjectIdentity` ya guarda la raíz que el puente resolvió del cwd en `default_project` (`server.rs:316-318`). Es la única raíz que el puente sabe legítima sin preguntar.
- `ClawgMcp::failed(...)` y `explain(&ClientError)` para devolver un error legible al modelo (`server.rs:723`, `:491`).
- El patrón de decisión pura separada de I/O: `resolve_project` recibe `resolve` por parámetro (`tools.rs:301-306`).
- `clawg sharing` y `sharing.json` como precedente de una política editada por el usuario y leída en cada llamada (`CLAUDE.md` del repo).
- `RootCollision` ya tiene su test a nivel de núcleo; falta el de punta a punta.

## Enfoques posibles

### Raíces registrables

**A. Cwd o ya registrada (recomendado como base).** Una raíz que no es la del cwd del puente y no está en `project` se rechaza, con un error que le dice al modelo qué hacer: pedirle al usuario que la agregue (`clawg projects add <ruta>`). La decisión es una función pura `(raíz, cwd_root, conocidas) -> Allow | NeedsConsent`.

- A favor: cierra el caso peligroso sin tocar el flujo normal. Un proyecto se vuelve «conocido» al abrir una sesión dentro de él o por consentimiento humano. El uso global (objetivo 2) no cambia: `all_projects` no pasa por la guarda.
- En contra: el uso entre proyectos legítimo («mirá cómo resolvió esto el repo B») pide un paso humano la primera vez. Un comando nuevo (`clawg projects add`) es superficie nueva.

**B. Cwd, registrada o raíz git.** Igual que A, pero además se permite cualquier ruta cuya raíz resuelta tenga `.git` (`identity.rs:55-68`).

- A favor: sin fricción para repos reales; bloquea `~`, `/`, `C:\` y carpetas sueltas.
- En contra: no hay consentimiento: cualquier repo del disco (incluido el de otro cliente, con secretos en Markdown) se indexa por pedido del modelo. Es una heurística, no una guarda.

**C. Lista editable (`projects.json`).** Como `sharing.json`: raíces permitidas por el usuario, con prefijos.

- A favor: control total del usuario, sin comando.
- En contra: otra fuente de verdad junto a `project`; archivos inválidos obligan a una regla de «qué pasa si está roto» (en `sharing.json` es «no comparte nada»; acá sería «no deja registrar nada»). Más superficie que A para la misma garantía.

**Recomendación:** A. B queda como alivio si la fricción molesta, y se puede añadir después sin romper nada: sólo agranda `Allow`.

### Alcance de la guarda

- **Sólo lo que lee disco** (`search_project`, y el refresco que dispara el registro). Las herramientas de memoria con una ruta desconocida registran la raíz **sin indexar**, o la rechazan igual. La separación hace falta porque hoy un `save_memory` con `project_path` desconocido también lanza el indexado (`api.rs:1036`, no hay distinción de quién registró).
- **Todo `project_path`.** Más simple y uniforme; un `save_memory` en otra ruta pide consentimiento. Rompe el test `all_projects_widens_a_search_beyond_the_bridges_own_project`, que guarda en `/some/elsewhere` (`server.rs:2279`); habría que cambiarlo a propósito.

La recomendación es lo primero: el riesgo es la lectura del disco, no la etiqueta.

### Dónde se aplica

- **En el puente** (decide con `default_project` y le pregunta al daemon qué está registrado): rápido, no cambia el protocolo, pero el daemon sigue aceptando cualquier raíz de quien tenga el token.
- **En el daemon** (`POST /v1/projects` con un campo que distinga «cwd del puente» de «pedida por el modelo»): la regla vive una sola vez. Quien tenga el token puede mentir en ese campo, igual que hoy puede registrar cualquier cosa.

Con cualquiera de los dos, **un agente con shell puede leer `~/.clawg*/token` y llamar al daemon directo**. La guarda protege el camino MCP; no es un límite de seguridad contra un agente con shell. Ese límite no existe hoy y este cambio no lo promete.

### Colisión

**A. Error al agente (recomendado).** El puente distingue el 409 de cualquier otro fallo de registro. En un 409 la herramienta devuelve un error legible: «el id X ya pertenece a <existing>; esta raíz no se puede usar hasta que se resuelva» y **no guarda ni busca**. Los demás fallos (daemon viejo, red) siguen siendo un warn, como pide R6.

- A favor: nada se mezcla. Un 409 es raro (ver abajo) y cuando ocurre, bloquear es más barato que contaminar.
- En contra: el usuario con dos `app` en conflicto no puede usar memoria en la segunda hasta que haya remedio. Con el hash de 16 bits el remedio no existe aún (ver abajo). Si se elige esta opción, `project-clone-identity` (30) debe entregar un id más ancho o asignado por el registro (su W2 o W3); con su W0 el usuario quedaría bloqueado sin salida. Cambia R6 de `specs/project` («un registro fallido no bloquea un save»): sólo para el 409.

**B. Degradar con aviso.** Seguir, pero sin escribir bajo ese id: la herramienta responde sin proyecto (como `all_projects`) y antepone una línea de aviso. Se usa la memoria global, no se mezcla, y el usuario se entera.

- A favor: no se pierde el uso.
- En contra: un save sin proyecto pierde la partición; los datos quedan en un lugar que el usuario no eligió.

**C. Resolver sola (id con sal).** Derivar un id alternativo para la segunda raíz.

- Ver «Descartado».

**Dónde se avisa por segunda vez.** El puente no reintenta tras un 409 (`registered` ya tiene la clave). Hay que decidir si el error se repite en cada llamada (recomendado: sí, el estado no cambia) o una vez por sesión.

**Cuánto vale esto.** Con `n` raíces del mismo nombre, el riesgo es `n(n-1)/2 / 65.536`: dos `app` dan 0,0015 %; diez `app`, 0,07 %. Es bajo y por eso el cambio es S. Pero cuando ocurre no se puede deshacer después, que es el motivo de gastar un cambio en hacerlo ruidoso.

### Dónde se ensancha el hash

**No acá.** Ensanchar el hash re-keya todas las filas de todos los proyectos (`project.rs:56-124` mueve fila por fila) y obliga a un snapshot previo que `register` hoy no toma (no hay llamada a `snapshot` en `api.rs:1017-1038`; los `Reason` son `PreMigration`, `PreRestore`, `Manual` y `PreReembed`, `backup.rs:119-128`). Eso es un cambio M con riesgo sobre la base del usuario. Va en `project-clone-identity`, que ya re-keya por otro motivo. 29 queda sin migración. Si 29 falla cerrado ante el 409, ese cambio tiene que ensanchar el id o hacer que lo asigne el registro (su W2/W3); si se queda con W0, hay que elegir en 29 la alternativa de degradar a sin proyecto.

## Descartado

- **Id con sal para la segunda raíz** (`app-1a2b-2`): rompe `is_project_id` (`{nombre}-{4 hex}`, `project.rs:40-49`), que valida los ids viejos de `POST /v1/projects` y evita inyectar texto a un nombre de tabla. Además el id depende del orden de llegada: la primera raíz registrada «gana» el id limpio, y si el usuario borra la base y vuelve a abrir en otro orden, los ids se invierten. Un id que cambia con el orden no es estable.
- **Rechazar sólo por nombre de carpeta parecido:** no detecta el choque; el 409 ya es la fuente de verdad, porque compara la raíz guardada.
- **Pedir confirmación al modelo con un campo `confirm: true`:** el modelo puede enviarlo solo. La confirmación tiene que salir de algo que el modelo no controla (un comando del usuario).
- **Ocultar `project_path` del schema de la herramienta:** sin él, el modelo no puede indicar el proyecto cuando el cwd no es el del trabajo (objetivo 2). Se necesita; se guarda, no se quita.
- **Validar el contenido de lo indexado** (más patrones de secretos): es otro problema (`redact`) y no cierra el hueco de qué directorio se lee.

## Alcance candidato

1. Función pura de decisión `(raíz, raíz del cwd, raíces conocidas) -> Allow | NeedsConsent` y su cableado en `project_for` para `search_project` (y para el registro que dispara indexado).
2. Consulta de raíces conocidas: `GET /v1/projects` (con token) que devuelve `(id, root_path)` de `project`, o un campo nuevo en `POST /v1/projects`. Decisión de diseño.
3. `clawg projects add <ruta>` (y `list`) para el consentimiento, por CLI, con el daemon corriendo, sin abrir la base fuera del daemon (como `clawg sharing`).
4. El 409 pasa de warn a error de herramienta, separado de los fallos no relacionados con el choque.
5. Rutas inexistentes no se registran (o se registran sin indexar).
6. Documentar en `CLAUDE.md` el comando nuevo y el error, y actualizar la descripción de `project_path` en los campos de las cinco herramientas.

### No-alcance

- Ensanchar el hash, cambiar la identidad o re-keyar filas (`project-clone-identity`).
- Separar filas ya mezcladas por una colisión previa (no hay dato para hacerlo).
- Un límite de seguridad contra agentes con shell.
- Validar las cabeceras `Host`/`Origin` del daemon y los permisos del archivo `token`: son de daemon-hardening (38).
- Permisos por agente sobre qué proyectos lee (`sharing.json` no habla de proyectos; cambiarlo es otro cambio).
- Un chequeo en reposo del estado de la base (`doctor-checks`).
- Cambios de ranking o de recuperación.

## Decisiones abiertas

Son del usuario.

- [ ] **Qué raíces se registran sin preguntar.** ¿Sólo cwd y las ya registradas (A, recomendado), también cualquier repo git (B), o una lista editable (C)?
- [ ] **Qué cubre la guarda.** ¿Sólo lo que lee disco (`search_project` y el indexado) o todo `project_path`? Si es lo primero, ¿una ruta desconocida en `save_memory` se registra sin indexar o se rechaza?
- [ ] **Cómo se da el consentimiento.** `clawg projects add <ruta>` por CLI (recomendado), un flag al arrancar `clawg mcp`, o sólo el mensaje al modelo para que pregunte.
- [ ] **Dónde vive la regla.** ¿En el puente, en el daemon con un campo nuevo en `POST /v1/projects`, o en ambos?
- [ ] **Qué hace la colisión.** ¿Error y no guardar (A), degradar a sin proyecto con aviso (B)? ¿Se repite el error en cada llamada?
- [ ] **Cambio de la regla R6.** `specs/project` R6 dice que un registro fallido no bloquea un save. ¿Se acota a «salvo 409»?
- [ ] **Qué pasa con raíces ya registradas hoy.** Todas las de `project` pasan a «conocidas» sin preguntar (recomendado: ya las eligió el usuario con su uso) o se reconfirman.
- [ ] **Rutas inexistentes.** ¿Error o registro sin indexar?
- [ ] **Si la guarda se apaga.** ¿Hay una variable o flag para quien usa el daemon con muchos repos y no quiere fricción?

## Cómo se mide

Tests RED primero (cada uno falla hoy):

1. Decisión pura: cwd root → `Allow`; raíz registrada → `Allow`; raíz desconocida → `NeedsConsent`; `all_projects` ignora la guarda.
2. `search_project` con un `project_path` desconocido devuelve un error que nombra `clawg projects add` y **no registra** ni lanza el indexado: `project` y `project_doc_file` siguen vacíos.
3. `search_project` con la raíz del cwd sigue funcionando sin consentimiento.
4. Tras `clawg projects add`, la misma llamada indexa y responde.
5. Colisión de punta a punta: sembrar `project` con el id de la raíz y otra `root_path`; un `save_memory` devuelve error y **no escribe** ninguna fila (hoy escribe 1, `server.rs:1198`); `search_project` no sirve documentos de la otra raíz.
6. Un fallo de registro que **no** es 409 (daemon viejo o sin red) sigue sin bloquear el save (fija el límite de R6).
7. Una ruta inexistente no deja fila en `project`.
8. Datos guardados: una base con raíces ya registradas por la versión anterior arranca con esas raíces como conocidas, sin tocar ninguna fila. No cambia el esquema ni el `project_id`; el test igual siembra la versión anterior porque `config.yaml` exige probar que lo guardado sigue alcanzable.
9. Los tests que cambian a propósito: `a_refused_registration_does_not_block_the_save` (`server.rs:1176`) y `all_projects_widens_a_search_beyond_the_bridges_own_project` si la guarda cubre todo `project_path` (`server.rs:2268`).

`rag-eval --diff`: no es requisito. No toca ranking ni recuperación, y el conjunto dorado de `--docs` usa la raíz del cwd.

## Riesgos

- **Fricción en el uso entre proyectos:** objetivo 2 vive del uso global. Si pedir consentimiento molesta, los usuarios lo apagan (decisión abierta) y la guarda no sirve. Por eso `all_projects` queda fuera.
- **Falsa sensación de seguridad:** un agente con shell lee el token y llama al daemon. Hay que decirlo en la documentación para que nadie venda esto como un límite.
- **Un 409 bloquea de verdad:** con A, quien choca no puede usar memoria en esa raíz hasta que exista el remedio (ancho del hash o `projects merge/rename`, en `project-clone-identity`). Para que el remedio exista, 30 debe entregar un id más ancho o asignado por el registro (W2/W3): `projects merge` no separa dos repos distintos con la misma carpeta, y W0 los dejaría bloqueados. Hay que mostrar el id y la raíz que ocupa para que el usuario entienda cuál es la otra.
- **Cambio de contrato de R6 y del texto de las herramientas:** hay tests y spec que lo fijan.
- **Cambio de herramientas MCP:** si el error nuevo cambia la descripción de `project_path`, hay que revisar los tests que fijan el schema (`server.rs`, test `the_viewer_is_not_exposed_to_the_model` y similares).
- **Doble lectura de `project`:** la lista de raíces conocidas se consulta en cada llamada o se cachea en el puente; un caché obsoleto rechaza una raíz recién agregada. Preferir consultar, el costo es un GET local.
