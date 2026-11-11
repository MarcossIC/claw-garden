# project-concept-map — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 | L, por pasos y con puerta | —; se apoya en el índice de docs de project-indexing (archivado); se coordina con [memory-anchors](../memory-anchors/exploration.md) (98) y alimenta a [project-architecture-map](../project-architecture-map/exploration.md) (126) | exploración (borrador); prioridad baja |

**Tesis.** El índice de un proyecto contesta una pregunta con pasajes, pero no guarda un mapa: nada lista los conceptos del proyecto, dónde vive cada uno ni cómo se relacionan. Quien pregunta «¿dónde está la funcionalidad de la que habla el usuario?» depende de que la búsqueda acierte con las palabras. Este cambio pregunta si vale la pena registrar un mapa de nodos (carpetas, módulos, conceptos e ideas) y relaciones entre ellos, que sirva para ubicar sin adivinar y como fuente de un dibujo del proyecto (126). No reabre lo que project-indexing descartó: lo mide con otra pregunta.

## El problema verificado contra el código

1. **El índice guarda pasajes sueltos.** Sus tablas son `project_doc_file`, `project_doc_chunk` (ruta, orden, breadcrumb y cuerpo), los vectores y el texto (`crates/clawg-core/migrations/V12__project_docs.sql:10-49`). No hay tabla de nodos ni de relaciones: un pasaje no sabe con qué otro se relaciona.
2. **El troceador de código recorre el árbol de sintaxis sólo para cortar.** `split` arma las piezas y `header` les pone el encabezado de la definición que las contiene (`crates/clawg-core/src/memory/project_docs/code.rs:142`, `:181`). Los nombres de las definiciones, los imports y las referencias se descartan. Cubre Rust, Python, TypeScript y TSX (`code.rs:34-39`).
3. **Lo que se descartó, y qué medía.** La [exploración de project-indexing](../archive/2026-10-15-project-indexing/exploration.md) descartó el mapa de símbolos, porque precargado resolvió 56 issues contra 66 sin índice, y expandir resultados por el grafo de llamadas, sin ganancia medida. El mapa generado por módulo quedó sin hacer porque pide un LLM y refresco. Las dos mediciones son de un agente resolviendo issues con el mapa precargado o con los resultados expandidos. Ninguna mide un mapa consultado a pedido, ni uno que lee una persona.
4. **La necesidad ya apareció en este repositorio.** [memory-module-layout](../memory-module-layout/exploration.md) armó a mano el grafo de dependencias de los 51 módulos de `clawg-core::memory` y encontró 32 en un solo ciclo. Ese análisis no lo repite ningún comando.
5. **clawg no llama a ningún LLM.** En `crates/clawg-core/src` y `crates/clawg-cli/src` ningún código nombra un proveedor ni lee una clave de uno. El único que puede nombrar un concepto es el agente, como hoy escribe las conclusiones que le pide `synthesize`.
6. **Topes del índice:** 2.000 archivos por proyecto y 256 KiB por archivo (`crates/clawg-core/src/memory/project_docs/corpus.rs:14-17`).

## Qué sería un nodo

No hace falta que el mapa sea un resumen por módulo. La idea es un grafo donde conviven lo que sale solo del árbol y lo que alguien nombra:

| Nodo | De dónde sale | Pide un LLM |
|---|---|---|
| Carpeta, archivo | el recorrido del índice | no |
| Módulo, definición | tree-sitter | no |
| Sección de un doc | los encabezados | no |
| Concepto, idea, funcionalidad | lo escribe el agente, o se deriva | sí (el agente), salvo en C |

| Relación | De dónde sale |
|---|---|
| contiene | la estructura |
| depende de | los `use` e `import` que tree-sitter ya parsea |
| vive en (concepto → archivo o definición) | el agente, o los identificadores que citan los docs y las memorias |
| se relaciona con (concepto ↔ concepto) | el agente |

## Enfoques

| Enfoque | Qué registra | A favor | En contra |
|---|---|---|---|
| A. Sólo estructura, derivada | carpetas, módulos y dependencias, en tablas derivadas como las del índice | Sin LLM, se reconstruye del disco, tree-sitter ya es dependencia | Es estructura, no comprensión: sola no contesta dónde está una funcionalidad |
| B. Conceptos que escribe el agente | un concepto con nombre, descripción y anclas a archivos o definiciones | Es lo que contesta la pregunta de fondo; el daemon puede listar qué zonas no tienen concepto, como `synthesize` lista memorias | Envejece cuando el código cambia (el problema de 98); cuesta tokens; la calidad depende del agente |
| C. Conceptos derivados sin LLM | agrupar pasajes por cercanía, o tomar como conceptos los encabezados de los docs y los dominios de `openspec/specs/` | Sin LLM ni costo | Un grupo no trae nombre, y no todo repositorio tiene docs que nombren sus conceptos |
| D. Importar el grafo de una herramienta externa | el `graph.json` de una herramienta como graphify | No se escribe el extractor | Una dependencia y un formato ajenos, y llamadas a un LLM que hace el asistente del usuario |

**Recomendado:** una puerta antes de escribir producto, y si pasa, A más B, con las anclas de 98 para saber cuándo un concepto quedó viejo. D sirve para armar el prototipo de la puerta, no como producto.

## Pasos propuestos

0. **Puerta, sin código del producto.** Para un repositorio real se arma el mapa a mano: la estructura con un script y unos 20 conceptos escritos por el agente. Se prueba con preguntas reales de «¿dónde está X?» y se compara con `search_project` solo.
1. **Estructura (A):** nodos y dependencias derivados, en una migración nueva, con el mismo refresco por hash del índice.
2. **Conceptos (B):** cómo se guardan, quién los escribe y cómo se marcan viejos.
3. **Consulta a pedido:** una herramienta que devuelve un nodo con sus vecinos. Nunca precargado, por la evidencia del punto 3.

## Cómo se mide

Fijado antes de mirar un resultado: un conjunto de preguntas de «¿dónde está X?» con el archivo correcto conocido. El mapa pasa si acierta en al menos 6 casos más que `search_project` solo y no pierde ninguno, que es la regla de `rag-eval --diff`. Si no pasa, el uso para el agente cierra sin código, y el mapa sólo sigue si 126 lo necesita para dibujar.

## Datos, migración y vuelta atrás

- La estructura (A) es derivada: se reconstruye del disco y no es memoria, como el índice de docs.
- **Un concepto que escribe el agente no es derivado.** Es un dato del usuario: entra en los snapshots, en el export y en la importación, y una versión nueva tiene que abrir los de la anterior.

## Fuera de alcance

- Dibujar el mapa: es 126.
- El grafo de llamadas, descartado en project-indexing.
- Un proveedor de LLM en el daemon.
- Precargar el mapa en el contexto del agente.

## Decisiones abiertas

- **Qué es un concepto en el store:** una memoria con un tipo propio, o una tabla aparte con sus relaciones.
- **Quién los escribe:** el agente a pedido, un comando que la persona corre, o derivados (C).
- **El grano:** por módulo, por funcionalidad o por idea.
- **Si se suman lenguajes** a los cuatro que el troceador ya parsea.

## Esfuerzo e impacto

| Paso | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|
| 0 | Hacer primero | Decide si el resto se hace | Bajo | Bajo |
| 1 | Si pasa la puerta | Medio: dependencias y ciclos sin armarlos a mano | Bajo (derivado) | Medio |
| 2 | Si pasa la puerta | Alto si los conceptos se mantienen al día | Medio (dato del usuario, envejece) | Medio–alto |
| 3 | Con 1 y 2 | Medio | Bajo | Bajo |

La puerta va en local: necesita un repositorio real y sesiones de un agente. Los pasos 1 a 3 se prueban en la nube con el embedder falso.

## Fuentes

- Código de `develop` en `f99e0211`, con las rutas citadas.
- La exploración de project-indexing, archivada el 2026-10-15, con sus mediciones y sus fuentes.
- https://pypi.org/project/graphifyy/ y artículos sobre graphify, leídos el 2026-11-11 a través de un resumen de búsqueda: extracción estructural por AST para el código y semántica por LLM para lo demás, con un `graph.html` interactivo de salida. Sin verificar contra su código.
