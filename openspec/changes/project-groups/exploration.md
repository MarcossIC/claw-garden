# project-groups — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 (separar las memorias por proyecto y también usarlas entre proyectos) | M–L, por partes (A es M; B y D son S; C es M; E es S y opcional; F se pospone) | —; se coordina con [knn-prefilter](../knn-prefilter/exploration.md) (37), porque leer a los hermanos cambia qué fracción de la ventana de vec0 se admite, y con [session-usage](../archive/2026-11-11-session-usage/exploration.md) (121), que podría reportar por grupo | exploración (borrador) |

**Tesis.** Una memoria de clawg tiene hoy dos alcances: un proyecto, o todos. No hay nada en el medio. Quien trabaja en diez repositorios de una misma empresa no puede guardar «en esta empresa los servicios se despliegan así» sin que esa fila aparezca también en sus proyectos personales y en los de otro cliente, y desde el repositorio `web` no llega a lo que se aprendió en `api` salvo buscando en todo. Lo que falta es una relación declarada y guardada entre proyectos: un **grupo** (la empresa, un producto, un área) que tiene proyectos y otros grupos adentro, que puede tener memorias propias, y que le dice a cada búsqueda qué proyectos son parientes. El alcance de lectura de varios proyectos a la vez ya existe; lo que no existe es dónde declararlo.

## El problema verificado contra el código

1. **`project` es una tabla plana.** Tiene `id`, `name`, `born`, `reviewed` y `created_at` (`crates/clawg-core/migrations/V27__project_registry.sql:29-35`), más `merged_into` (`V28__project_merge.sql:8`) y `deleted_at` (`V29__project_delete.sql:7`). Ninguna columna relaciona dos proyectos vivos.
2. **Las relaciones que hay entre proyectos dicen otra cosa.** `project_suggestion` dice que dos pueden ser el mismo (`V27__project_registry.sql`), `merged_into` que lo son, y `project_subfolder` que una subcarpeta de un repositorio es un proyecto aparte (`V30__project_subfolder.sql:10`). Esta última es la única con un padre y un hijo, y no comparte nada: desde un subproyecto no se leen las memorias de su repositorio ([`specs/project`](../../specs/project/spec.md), PL52).
3. **Lo que lee una búsqueda con proyecto.** El predicado admite las filas de una lista de proyectos, las que no tienen proyecto y toda `user_preference` (`memory_project_predicate!`, `crates/clawg-core/src/memory/search.rs:114-121`). Lo único que cruza proyectos es lo que cruza a todos.
4. **Leer varios proyectos a la vez ya funciona, pero el grupo es efímero.** `SearchScope.projects` es una lista (`search.rs:29`, `with_projects` en `:55`), `POST /v1/search` toma `project_ids` (`SearchRequest`, `crates/clawg-cli/src/daemon/api.rs:424`; `searched_projects`, `:1590`) y la búsqueda es una sola, con un ranking ([`specs/retrieval`](../../specs/retrieval/spec.md), BI6). Ese grupo lo arma el puente en cada llamada con las carpetas que el cliente tiene abiertas (`open_projects`, `crates/clawg-cli/src/mcp/server.rs:634`). Depende de cómo se abrió el editor: un cliente con una sola carpeta abierta lee un solo proyecto, aunque sea uno de diez repositorios de la misma empresa.
5. **El contexto es de un proyecto.** `POST /v1/context` toma un solo `project_id` (`ContextRequest`, `api.rs:445`), y con carpetas de varios proyectos abiertas `recall_context` se niega y pide cuál ([`specs/mcp`](../../specs/mcp/spec.md), BI7).
6. **Un save va a un proyecto o a todos.** Un save nombra `project_path`, `project` o `all_projects` (BI4). `all_projects` deja la fila sin proyecto, y el predicado del punto 3 la muestra desde cualquier proyecto. No hay cómo decir «vale para estos proyectos y no para los demás».
7. **Las reglas de un save comparan dentro del proyecto exacto.** La deduplicación (`crates/clawg-core/src/memory/dedup.rs:186`) y la clave de tema (`crates/clawg-core/src/memory/topic.rs:88`) comparan `project_id` por igualdad, y el detector de contradicciones busca en el proyecto del save (`contradiction_search_scope`, `crates/clawg-core/src/memory/contradiction.rs:168-171`).
8. **Un hit ya sabe decir de qué proyecto viene.** Con varios proyectos en juego, cada hit muestra ` project=<id>` (`display_project`, `crates/clawg-cli/src/mcp/tools.rs:207`; BI5). El marcador que pide «saber de dónde surge» existe; hoy sólo aparece con varias carpetas abiertas.
9. **El registro ya guarda una señal de grupo.** La clave de un remoto es `host/dueño/repositorio` (`normalize_remote`, `crates/clawg-core/src/memory/identity.rs:610`), en `project_remote`. Los proyectos de un mismo dueño son candidatos a un mismo grupo, sin leer nada nuevo del disco.

## El nombre

«Workspace» ya nombra tres cosas en este repositorio:

- el archivo `workspace.md` de cada agente y proyecto (`workspace_path`, `crates/clawg-core/src/memory/workspace.rs:71`) y el bloque `=== WORKSPACE ===` que el modelo lee en cada contexto (`SEP_WORKSPACE`, `crates/clawg-core/src/memory/prompt_builder.rs:24`);
- la ventana de varias carpetas de un editor, en los escenarios de BI5 y BI7;
- el workspace de Cargo.

Un cuarto sentido, y justo en lo que ve el modelo, confunde. **Recomendado: «grupo» (`group`)**, que además es el nombre que BI6 ya le da a «varios proyectos leídos a la vez»: un grupo declarado es ese mismo grupo, guardado. Es una decisión abierta.

## Enfoques para el modelo

| Enfoque | Qué es | A favor | En contra |
|---|---|---|---|
| **(a) Tabla de grupos** | `project_group` (id, nombre, padre) y `project.group_id`; las memorias del grupo en una columna nueva `memory.group_id` | El grupo y el proyecto no se confunden | Una columna nueva en `memory` y un predicado nuevo en cada consulta que hoy usa `memory_project_predicate!`. Hay que repetir para los grupos lo que los proyectos ya tienen: referencia por nombre, renombrar, unir, borrar con su vuelta atrás, mover filas, exportar |
| **(b) Un grupo es un proyecto con hijos** | `project.parent_id` nombra otro proyecto. El padre suele ser un proyecto sin carpeta, que ya existe (PL8, PL33) | Una columna, y ninguna en `memory`. Guardar en el grupo es `save_memory` con `project: "<grupo>"`, que ya funciona (PL34). Pasar una memoria de un proyecto a su grupo es el movimiento que ya existe (R8, `to_project`). El predicado SQL no cambia: el daemon le ata la lista «el proyecto y sus ancestros». Renombrar, unir, borrar y exportar ya están | «Proyecto» pasa a nombrar también a un contenedor, y el listado tiene que mostrarlo. Hay que impedir ciclos y decidir qué pasa con los hijos de un padre que se une o se borra |
| **(c) Etiquetas** | `project_tag`, de muchos a muchos, sin jerarquía | Un proyecto en varios grupos | No hay subgrupos, que es lo pedido. Con varios grupos por proyecto, no queda claro de cuál hereda ni con cuál se compara un save |

**Recomendado: (b).** Reusa casi todo lo que el registro de proyectos ya resolvió, y deja `memory` como está. Un repositorio también puede ser padre: el de un monorepo, de sus subproyectos declarados.

**Un padre o varios.** Recomendado uno: un árbol. Con uno, «los ancestros de P» es una cadena y lo que P hereda no depende de un orden. Lo que queda afuera (una biblioteca que usan dos empresas) se sigue alcanzando con `project` y `all_projects`.

## Partes

### A. El árbol (Incorporar; M)

- **Esquema:** una migración que agrega `project.parent_id`, NULL en cada fila. No cambia ninguna fila ni ninguna otra tabla.
- **Lectura por lápida:** el padre se lee siempre por `canonical` (`crates/clawg-core/src/memory/merge.rs:54`), como una declaración de subcarpeta (PL45), así una unión no reescribe a los hijos.
- **Reglas, en una función pura:** un proyecto no es su propio ancestro, la cadena tiene un tope de profundidad (propuesto 8), y un padre borrado o inexistente se rechaza.
- **CLI, por el daemon que corre:** poner y quitar el padre de un proyecto, y `clawg projects list` con el árbol. Un grupo se crea con `clawg projects create`, que ya existe. Los nombres de los subcomandos quedan abiertos.
- **`GET /v1/projects`:** cada proyecto con su padre y sus hijos.
- **Export e import:** la línea `project` suma el padre; un lector anterior ignora el campo, como ignora los demás (PL54).
- **`doctor`:** un chequeo que nombra un padre que no está en `project` y un ciclo.

### B. Lo del grupo se lee desde abajo (Incorporar; S)

Una búsqueda, un contexto y una síntesis de un proyecto P leen también las filas de sus ancestros. El daemon arma la lista (P y su cadena) antes de atarla; el predicado y el ranking son los de hoy. Cada hit de un ancestro lleva ` project=<id>`, así el agente ve que esa fila es del grupo y no del proyecto.

Es el caso «convenciones de la empresa»: se guardan una vez en el grupo y llegan a cada proyecto de abajo, y a ningún otro.

Queda por decidir qué hacen las reglas del punto 7:

- **Deduplicación y clave de tema:** recomendado que sigan en el proyecto exacto. Una fila del proyecto con la clave de una del grupo es una excepción local, no una revisión de la del grupo.
- **Detector de contradicciones:** recomendado que sus candidatas incluyan a los ancestros, para que «acá se indenta con tabs» se empareje con «en la empresa se indenta con espacios». La propuesta tiene que leer qué hace hoy un par entre dos proyectos (`same_scope`, `contradiction.rs:219`) antes de fijarlo.

### C. Leer a los hermanos, a pedido (Incorporar; M)

`search_memory` suma un argumento (propuesto `group: true`) que lee el subárbol de la raíz del proyecto: el proyecto, sus ancestros, sus hermanos y los hijos de todos. Es la búsqueda de grupo de BI6 con la lista que da el árbol en vez de las carpetas abiertas, y cada hit nombra su proyecto.

**No es el alcance por defecto**, por dos razones:

- Un proyecto chico con hermanos grandes quedaría tapado: los filtros de proyecto corren después del corte k de vec0 ([knn-prefilter](../knn-prefilter/exploration.md), 37), y con más proyectos admitidos compiten más filas por los mismos diez lugares.
- `rag-eval` no lo puede medir: su golden tiene un solo agente y ningún proyecto. Un cambio de ranking por defecto sin medición no entra ([`config.yaml`](../../config.yaml), reglas de diseño).

Con un proyecto que es un grupo, `project: "<grupo>"` lee sólo las filas del grupo, como hoy; con `group: true`, también las de sus hijos.

### D. El contexto dice dónde está parado (Incorporar; S)

`recall_context` y el arranque de sesión suman una línea con la cadena de grupos del proyecto y sus hermanos, con un tope. Sin eso el agente no sabe que existe `api` al lado de `web` ni que vale la pena buscar con `group`. Cuesta tokens en cada sesión, así que la línea es una y se mide contra la línea base de agent-view.

### E. Sugerir grupos (Opcional; S)

Con el dueño del remoto (punto 9) o una carpeta padre común, `clawg projects list` nombra los proyectos que podrían ir juntos. Nunca agrupa solo: como las sugerencias de proyectos repetidos, deja la pregunta y la persona decide.

### F. Relaciones entre pares (Posponer)

«`web` consume la API de `api`» es otra relación: no es pertenencia, tiene dirección y tipo. El árbol no la expresa. Queda afuera hasta que la parte C muestre si leer a los hermanos alcanza.

## Datos, migración y vuelta atrás

- La migración es aditiva. El test que pide [`config.yaml`](../../config.yaml) siembra un store en la versión anterior (proyectos vivos, una lápida, un borrado abierto, declaraciones de subcarpeta y filas vivas y borradas) y comprueba que ninguna fila cambió y que cada una se alcanza como antes.
- Sin ningún padre declarado, cada respuesta es byte a byte la de hoy. La suite actual lo fija sin tocarla, salvo los contratos del agente, que se bendicen por el argumento nuevo.
- La vuelta atrás es el snapshot `pre-migration`. La propuesta tiene que decir qué hace un binario anterior ante el esquema nuevo y cómo se vuelve.
- Ninguna memoria cambia de proyecto por agrupar. Agrupar y desagrupar sólo escriben `parent_id`.

## Cómo se mide

Tests que hoy fallan:

1. **Herencia.** Filas en `acme` (grupo), en `web` y `api` (sus hijos), en `otro` (sin grupo) y una sin proyecto. Una búsqueda desde `web` trae la de `web`, la de `acme` con su `project=` y la sin proyecto; no la de `api` ni la de `otro`.
2. **Hermanos.** La misma búsqueda con `group` trae además la de `api`, con su `project=`, y nunca la de `otro`.
3. **Subgrupos.** Con `acme > plataforma > api`, desde `api` se lee lo de `plataforma` y lo de `acme`; desde `web`, hijo directo de `acme`, no se lee lo de `plataforma`.
4. **Ciclo.** Poner a `acme` bajo `api` se rechaza sin escribir.
5. **Unión.** Unir `acme` en `acme-corp` deja a sus hijos leyendo lo de `acme-corp`, y el `--undo` los devuelve.
6. **Export.** Un export y su import en un store vacío reconstruyen el árbol.

`rag-eval --diff` contra la línea base no tiene que mover ningún caso: es una puerta, no una evidencia, porque el golden no tiene proyectos. Si la parte C se propone alguna vez como alcance por defecto, necesita antes un corpus con proyectos, como el de vectores armados de 37 (`crates/clawg-core/src/memory/crowding.rs`).

## Fuera de alcance

- **`sharing.json`:** decide qué agente lee a qué agente, y no cambia. Un grupo no es una frontera de acceso, igual que un proyecto no lo es.
- **`workspace.md`, LAST SESSION y RECENT:** siguen siendo del proyecto y del agente.
- **Los docs indexados:** `search_project` sigue buscando en una carpeta.
- **Que el agente agrupe proyectos.** Agrupar es una decisión de la persona, por CLI. El agente ve el árbol y guarda en un grupo por su nombre.
- **Grupos compartidos entre máquinas o personas.** El árbol viaja en un export, y nada más.

## Decisiones abiertas

- **El nombre:** `group`, recomendado, o `workspace`.
- **El modelo:** (b), recomendado, o (a).
- **Un padre o varios.** Recomendado uno.
- **Si lo del grupo se lee desde abajo sin pedirlo** (parte B). Recomendado sí: una fila guardada en el grupo se guardó para eso.
- **Si un subproyecto de un monorepo tiene a su repositorio como padre sin declararlo.** Recomendado no: PL52 fijó que no lee lo del repositorio, y cambiarlo solo le cambiaría los resultados a quien ya los usa. Se declara como cualquier otro padre.
- **Qué pasa al borrar un padre con hijos vivos.** Recomendado: se rechaza nombrándolos, hasta que se muevan o se desagrupen.
- **Las reglas del save frente a los ancestros** (parte B).
- **Los nombres de los subcomandos** y si `group` es un booleano o nombra el grupo.

## Esfuerzo e impacto

| Parte | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|
| A | Incorporar | Alto: es la base del resto | Medio (toca el esquema de `memory.db`, de forma aditiva) | Medio |
| B | Incorporar | Alto: es el alcance intermedio que falta | Medio (suma filas a cada búsqueda de un proyecto agrupado) | Bajo |
| C | Incorporar | Medio | Bajo (sólo a pedido) | Medio (contrato del agente en los dos idiomas) |
| D | Incorporar | Medio: sin esto el agente no usa C | Bajo, con el costo de tokens medido | Bajo |
| E | Opcional | Bajo | Bajo | Bajo |
| F | Posponer | — | — | — |

Va en la nube: todo se prueba en Linux con el embedder falso y un daemon de test. Lo único local es agrupar los proyectos de un store real y mirar una semana de búsquedas.

## Fuentes

- Código de `develop` en `0165316`, con las rutas citadas.
- [`specs/project`](../../specs/project/spec.md) (PL8, PL33, PL34, PL45, PL52, PL54), [`specs/mcp`](../../specs/mcp/spec.md) (BI3 a BI7) y [`specs/retrieval`](../../specs/retrieval/spec.md) (BI6).
