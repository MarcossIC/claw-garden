# project-architecture-map — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 (para una persona: ver y analizar el proyecto) | sin estimar: lo decide esta exploración | [project-concept-map](../project-concept-map/exploration.md) (125), de donde salen los nodos y las relaciones | exploración (borrador); es una pregunta de viabilidad; prioridad baja |

**Tesis.** clawg le habla a un agente y a una terminal, siempre con texto: nada dibuja un proyecto. La idea es un mapa visual del proyecto, con sus carpetas, módulos y conceptos y cómo se relacionan, o un mapa de su arquitectura que se pueda mirar y analizar para encontrar fallas, malas prácticas y mejoras. Este documento no propone un diseño. Deja escritas las preguntas que hay que contestar cuando se revise: si es viable, qué se podría armar, qué información falta registrar y en qué formato se revisaría.

## El problema verificado contra el código

1. **Ninguna salida es gráfica.** Los comandos imprimen texto o JSON ([commands.md](../../../docs/reference/commands.md)), y en `crates/` no hay nada que escriba HTML: las dos apariciones son cuerpos de error de prueba (`crates/clawg-cli/src/client.rs:2152`, `:2269`).
2. **Los datos para dibujar no existen.** El índice guarda pasajes sin relaciones, y el troceador descarta nombres, imports y referencias (puntos 1 y 2 de 125).
3. **El análisis de arquitectura ya se hizo a mano en este repositorio, tres veces:**
   - [memory-module-layout](../memory-module-layout/exploration.md): 32 de 51 módulos en un solo ciclo, y dependencias invertidas;
   - [store-boundary](../store-boundary/exploration.md): 44 de 51 módulos `pub`, que le esconden código muerto al compilador;
   - [core-use-cases](../core-use-cases/exploration.md): `daemon/api.rs` con más de 3.000 líneas y handlers que deciden reglas de dominio.

   Son el tipo de hallazgo que un mapa podría dar sin rearmarlo cada vez.
4. **Parte de ese análisis ya se fija como test, no como dibujo.** `crates/clawg-cli/tests/conventions.rs` falla ante ciertas faltas, y memory-module-layout propone fijar con un test la dirección permitida de las dependencias.

## Preguntas a contestar

### 1. ¿Es viable?

Depende de tres cosas que hoy no se saben:

- **Si 125 llega a registrar nodos y relaciones.** Sin eso no hay qué dibujar.
- **El tamaño.** Un índice admite hasta 2.000 archivos (`crates/clawg-core/src/memory/project_docs/corpus.rs:17`). Un grafo de ese tamaño no se lee sin agrupar por carpeta o módulo y abrir de a un nivel.
- **Si le toca a clawg.** La meta de esta etapa es ser la memoria de los agentes ([ROADMAP](../../ROADMAP.md)). Un dibujo para una persona es otra clase de producto, y un desenlace válido es cerrar sin código y recomendar una herramienta externa.

### 2. ¿Qué se podría armar?

| Opción | Qué muestra | Qué pide |
|---|---|---|
| a. Mapa de carpetas y módulos con sus dependencias | Ciclos, dependencias en la dirección equivocada, módulos de los que depende todo, archivos enormes | Sólo la estructura de 125 (su enfoque A), sin LLM |
| b. Mapa de conceptos sobre la estructura | Qué funcionalidad vive en qué módulos, y qué conceptos comparten código | Los conceptos de 125 (su enfoque B) |
| c. Arquitectura declarada contra la real | Las capas o dominios que el proyecto dice tener, y cada dependencia que los viola | Que alguien declare las capas y las direcciones permitidas, en un archivo del proyecto |
| d. Reporte de hallazgos, en texto | La lista de ciclos, módulos centrales, archivos huérfanos y archivos sobre un tamaño | Lo mismo que a; no pide dibujar |

La opción d contesta buena parte del «analizar fallas» sin ninguna pieza gráfica, y la puede leer el agente.

### 3. ¿Qué información falta registrar?

- Las dependencias entre archivos (`use`, `import`), que el troceador parsea y descarta.
- Los nombres de las definiciones y a qué módulo pertenecen.
- El tamaño de cada archivo y módulo, que el recorrido ya conoce y no guarda.
- La arquitectura declarada: capas, dominios y direcciones permitidas. Hoy no está en ningún formato que una máquina lea.
- Las anclas de cada concepto a su código (125) y de cada memoria a los archivos que cita ([memory-anchors](../memory-anchors/exploration.md), 98), para mostrar qué se sabe de cada zona.
- Quizás la historia: qué archivos cambian juntos. Sale de git y no está evaluado.

### 4. ¿En qué formato se revisa?

| Formato | A favor | En contra |
|---|---|---|
| JSON de nodos y relaciones | Neutral: lo dibuja cualquier herramienta, y es la base de los demás | Solo, no se mira |
| Mermaid dentro de Markdown | Se ve en GitHub y en los editores, se compara en un diff, no suma dependencias | Sólo grafos chicos, de unas decenas de nodos |
| Graphviz DOT | Dibuja grafos más grandes | Pide una herramienta instalada para verlo |
| Una página HTML suelta e interactiva | Es lo que sirve para explorar un grafo grande: buscar, filtrar, abrir un grupo | Lleva una biblioteca de JavaScript adentro o la baja de una CDN, y es una superficie nueva que mantener |
| Texto para el agente, por una herramienta | Sirve al análisis sin dibujar | No es un mapa visual |

Para evaluar primero: el JSON más Mermaid por nivel (carpeta, después módulo), porque ninguno suma una dependencia. La página HTML sólo si esos dos no alcanzan.

### 5. ¿Quién analiza?

clawg puede dar hechos sin opinar: hay un ciclo, este módulo depende de aquel, este archivo tiene tantas líneas. Decir que algo es una mala práctica o proponer una mejora es un juicio, y clawg no tiene un LLM propio: lo hace el agente o la persona leyendo el mapa. El límite entre lo que el producto afirma y lo que deja a quien lee es parte de la decisión.

## Cómo se mide

Fijado antes de armar nada: con el mapa a mano de la puerta de 125, se dibuja este repositorio en dos formatos y se comprueba si muestra los tres hallazgos del punto 3, que ya están documentados. Si el mapa no deja ver lo que se encontró a mano, no paga su costo.

## Fuera de alcance

- Una interfaz web servida por el daemon.
- Editar el mapa arrastrando nodos.
- Un grafo de varios repositorios a la vez.

## Decisiones abiertas

- **Si se hace en clawg** o se recomienda una herramienta externa sobre el JSON.
- **Cuál de las cuatro opciones va primero.** La d es la más barata y la única que no dibuja.
- **El formato.**
- **Dónde se declara la arquitectura** de un proyecto (opción c), y quién la escribe.

## Esfuerzo e impacto

Sin estimar. La exploración se retoma cuando 125 tenga su puerta medida: antes de eso no hay datos sobre los que decidir.

## Fuentes

- Código de `develop` en `f99e0211`, con las rutas citadas.
- Las exploraciones de memory-module-layout, store-boundary y core-use-cases, de donde salen los tres hallazgos.
