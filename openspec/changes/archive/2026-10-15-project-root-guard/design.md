# project-root-guard — Diseño

Las decisiones de la [propuesta](proposal.md), con las alternativas que se descartaron. Las más amplias (lista editable, raíz git, ids con sal) están en la [exploración](exploration.md#enfoques-posibles).

## D1. Dónde vive la regla

**Elegido:** en el daemon, con una marca en `project`. El puente sólo dice si la raíz es la suya; el daemon guarda la marca, la consulta antes de leer el disco y responde con el remedio.

| Alternativa | Por qué no |
|---|---|
| Sólo el puente: consulta las raíces registradas (`GET /v1/projects`) y no registra una ajena | Para no abrir el hueco, una raíz ajena no puede entrar a `project` ni por un save, y entonces su registro no corre: las filas viejas guardadas bajo el id viejo de esa ruta dejan de pasar solas al id de la raíz (`project.rs:89-105`). Además suma una ruta y una consulta por llamada, y la regla queda en un proceso que no es el que lee el disco |
| Un archivo como `sharing.json` | Otra fuente de verdad junto a `project`, y una regla para el archivo inválido. Lo descartó la exploración (enfoque C) |
| Un campo «quién registra» sin guardar nada | El daemon tendría que recordar entre llamadas qué raíz aceptó; eso es la marca |

Quien tenga el `token` puede mandar `index_docs: true` por cualquier raíz, igual que hoy puede registrar cualquiera. No es un límite contra un agente con shell (fuera de alcance).

## D2. La marca y su migración

```sql
-- V13
ALTER TABLE project ADD COLUMN index_docs INTEGER NOT NULL DEFAULT 1;
```

- **Default 1:** las raíces que ya existen quedan aceptadas sin pasar por cada fila, y las inserciones de los tests que no nombran la columna (`crates/clawg-cli/tests/api.rs:1631`) siguen dando una raíz indexable. El código de producción siempre la nombra (`register_project`).
- **Sin `CHECK`:** la única escritura es `register_project`, que guarda `0` o `1` desde un `bool`.
- **Rollback a mano:** `ALTER TABLE project DROP COLUMN index_docs; DELETE FROM refinery_schema_history WHERE version = 13;` (SQLite 3.35+, el que trae `rusqlite` con `bundled`). El camino documentado es `clawg restore` del snapshot `pre-migration`.

| Alternativa | Por qué no |
|---|---|
| Tabla aparte `project_docs_consent (id)` | Dos tablas que dicen lo mismo sobre una raíz; un join en cada lectura |
| Default 0 y marcar las existentes con un `UPDATE` | Mismo resultado con una sentencia más, y las inserciones de test quedarían sin indexar |

## D3. El registro

```rust
pub async fn register_project(store, workspaces, root, legacy_ids, index_docs: bool)
    -> Result<RegisterOutcome, RegisterError>   // RegisterOutcome { project_id, rekeyed, index_docs }
```

Dentro de la transacción de hoy: `INSERT OR IGNORE ... (id, root_path, created_at, index_docs)`; leer `root_path, index_docs`; si la raíz difiere, `RootCollision` como hoy y sin escribir; si no, y se pidió indexar y estaba en 0, `UPDATE project SET index_docs = 1`. El resultado lleva el valor final, que es lo que decide `spawn_doc_refresh` en `api.rs`.

La raíz que no es un directorio con `index_docs: true` se rechaza en el handler (400) antes de `register_project`: es una comprobación de disco y el núcleo no la necesita para nada más.

## D4. Quién se niega a leer

`root_of` (`project_docs/mod.rs:426`) pasa a leer también `index_docs` y devuelve un error nuevo, `DocIndexError::NotAccepted(root)`, cuando es 0. Como `try_refresh` empieza por `root_of` (`mod.rs:221`), ningún camino lee el disco de una raíz no aceptada, aunque un caller se olvide de mirar la marca. `search_docs` mapea `NotAccepted` a 403 y `UnknownProject` a 404, como hoy.

| Alternativa | Por qué no |
|---|---|
| Mirar la marca sólo en el handler de búsqueda | `spawn_doc_refresh` y cualquier caller futuro de `try_refresh` tendrían que acordarse; la guarda tiene que estar junto a la lectura |
| 404 para una raíz no aceptada | El puente ya trata un 404 «pelado» como daemon viejo (`server.rs:818-823`); un 403 dice lo que es |

El texto del 403 lo arma el daemon, que sabe su perfil (`AppCtx::profile`, `api.rs:76`), con una función pura que recibe raíz y perfil. Así también lo lee bien un puente anterior a este cambio (que lo muestra como `daemon returned 403: …`).

## D5. El puente

- `indexes_docs(project: &ProjectIdentity, bridge: Option<&ProjectIdentity>) -> bool` en `tools.rs`, pura: compara `root`. Se compara la raíz y no el id porque dos raíces que chocan dan el mismo id.
- `project_for` pasa a devolver `Result<Option<String>, String>`: `Err` es el texto de la colisión. En un 409 saca la clave de `registered` (como ya hace en un fallo que el daemon no vio, `server.rs:571-573`) para que la llamada siguiente pregunte de nuevo. Cada herramienta convierte el `Err` en `Self::failed`.
- `search_project_impl` gana un brazo para `Daemon { status: 403, .. }` que devuelve el `message` tal cual.

| Alternativa | Por qué no |
|---|---|
| Recordar la colisión y no volver a preguntar | Un `clawg restore` o el arreglo de 30 la resuelven sin que el puente se entere; preguntar cuesta un POST local en un caso que es raro |
| Degradar a sin proyecto con un aviso (enfoque B) | El save queda en un lugar que el usuario no eligió, y un aviso en el texto de éxito se lee como éxito |

## D6. `clawg projects add`

Un `Command::Projects { action: ProjectsAction }` con un único `Add { path, profile }`, para que `list` y `remove` entren después sin cambiar la forma. La ejecución sigue a `clawg undelete` (`crates/clawg-cli/src/main.rs:168-179`): `probe_running` sin arrancar, y el texto de `memories::explain_unavailable` si no hay daemon. La lógica (resolver, comprobar el directorio, armar el pedido y el texto de salida) vive en `crates/clawg-cli/src/projects.rs`, testeable contra un daemon en proceso.

`DaemonClient::register_project` gana el parámetro `index_docs: bool`.
