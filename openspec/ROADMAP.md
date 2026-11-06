# Roadmap

Qué sigue y en qué orden. Es el único lugar que ordena lo pendiente: [README.md](README.md) es el mapa de lo que existe y cada cambio guarda su detalle en su carpeta. Lo terminado sale de acá.

**La meta de esta etapa:** clawg es la única memoria de los clientes de agentes de uso diario, Claude Code y Cursor, en todos los proyectos. Lo que no acerca esa meta espera, aunque sea buena idea.

## Cómo se ordena

1. **Lo que daña si se instala antes, va antes.** Un contexto malo inyectado en cada sesión es peor que no inyectar nada.
2. **Primero un repositorio real, después todos.** Lo que se instala a nivel usuario afecta a todos los proyectos, y Cursor también lee esa configuración.
3. **Los datos se tocan una sola vez y al final,** después de ensayarlos sobre una copia.
4. **Lo que sólo vale si un problema aparece, espera a que aparezca.** Esos cambios tienen su condición escrita abajo.

## Local o nube

**Va en local sólo si necesita** alguna de estas cosas:
- el store real o una copia, un export o las transcripciones de los clientes;
- un cliente instalado, como Cursor con sus ventanas;
- una conducta de Windows que el CI no reproduce, o medir contra una línea base de esa máquina;
- instalar o publicar, que hace la persona.

**Todo lo demás va en la nube:** lo que se prueba en Linux con el embedder falso y termina en un PR a `develop`. Lo que un cambio de nube no pueda comprobar queda en su `verify.md` como pendiente en local.

Una noche lleva como mucho una tarea local y dos de nube que no toquen los mismos archivos.

## Noche 1

| Cambio | Dónde | Necesita antes |
|---|---|---|
| [bridge-project-identity](changes/bridge-project-identity/exploration.md) | Nube | — (la [prueba con Cursor real](changes/bridge-project-identity/exploration.md#resultado-de-la-prueba-con-cursor-2026-11-05-y-2026-11-06) está hecha) |

## Noche 2

| Cambio | Dónde | Por qué acá |
|---|---|---|
| [retire-legacy-kinds](changes/retire-legacy-kinds/exploration.md) | Nube | Saca dos KNN de cada búsqueda y borra tablas: conviene que salga solo y antes del corte |
| Partir el mapa de openspec | Nube | Lleva las filas archivadas a `changes/archive/README.md`. Conserva el encabezado «Orden de ejecución de lo pendiente» y su razonamiento, que el caso d13 de `crates/clawg-eval/golden/project-docs.json` espera, o actualiza ese caso |

## Piloto (una semana de trabajo real, no una noche)

Se publica lo anterior y se instalan los hooks a mano en un repositorio real con memorias importadas, en `.claude/settings.local.json`, junto con Cursor en el mismo repositorio. No se usa `clawg setup` todavía, porque escribe a nivel usuario, con una excepción: Cursor se conecta con `clawg setup cursor --profile prod`, porque esa es la prueba con Cursor real ([pendiente en prod](changes/archive/2026-11-04-client-setup-cursor/verify.md#pendiente-en-prod)). Esa instalación también es a nivel usuario y afecta a cada ventana de Cursor, no sólo a este repositorio. Los mismos hooks prueban el arranque real de session-start-context ([pendiente en prod](changes/archive/2026-11-04-session-start-context/verify.md#pendiente-en-prod)), y que una preferencia declarada a Claude Code entre al arranque de Cursor en el mismo proyecto ([preference-model](changes/archive/2026-11-06-preference-model/verify.md#pendiente-en-prod)).

**Pasa si:**
- responde 10 preguntas reales cuya respuesta sólo está en el historial;
- al abrir una sesión, el contexto trae entero el resumen de la sesión anterior;
- `clawg doctor` no da errores y `ingest.failed` queda en 0;
- una memoria guardada desde Cursor aparece en Claude Code y al revés, en el proyecto correcto;
- ninguna ventana se abre sola;
- la RAM es aceptable: esta semana decide el brazo B de ram-footprint-gates, porque cada evento de hook despierta el modelo.

## Corte (una noche local, con el piloto aprobado)

Antes, lo que dejó el ensayo de history-reimport en [«Para el corte»](changes/history-reimport/verify.md#para-el-corte): decidir qué snapshots automáticos de `prod` se copian aparte, porque el corte poda algunos, y volver a clasificar las notas de un export nuevo que el mapa no conoce.

Paso 2 de [history-reimport](changes/history-reimport/exploration.md), y después:

1. `clawg setup claude-code --remove` y `clawg setup claude-code`, porque el servidor ya registrado sin `--agent` no se actualiza solo. Después `clawg setup cursor`.
2. Sacar el plugin de Engram, su bloque de protocolo del `CLAUDE.md` global y su servidor de Cursor.
3. Decidir dónde guarda sus artefactos el flujo SDD de los skills de usuario: hoy usa Engram por omisión. Recomendado: `openspec`, que ya existe y no depende de una herramienta de memoria. No actualizar Engram antes del último export: clawg lee los formatos `0.1.0` y `0.2.0`.
4. Decidir si hace falta la captura de `Stop` y `SubagentStop`, que Engram tiene y clawg no.
5. Mirar el `--dry-run` de `clawg import --from claude-code` y decidir.
6. Dejar Engram instalado y sin escrituras dos semanas, como vuelta atrás. Después, se desinstala.

## Sube sólo si el piloto lo pide

| Cambio | Sube si |
|---|---|
| [knn-prefilter](changes/knn-prefilter/proposal.md) | Las búsquedas en un proyecto chico traen poco o nada que sí está guardado. La fracción de filas ajenas es lo que decide la pérdida |
| [read-connections](changes/read-connections/exploration.md) | Un hook vence sus 3 s detrás de una operación larga y el arranque queda sin contexto |
| [session-lifecycle](changes/session-lifecycle/exploration.md) | Dos sesiones en paralelo del mismo proyecto reciben el traspaso de la otra |
| [agent-eval](changes/agent-eval/proposal.md) | El agente no busca ni guarda cuando debería, y hay que medirlo. Tareas, fixtures y corredor listos para sesiones pagas; falta correr el piloto en local, con una clave de API ([tareas](changes/agent-eval/tasks.md#después-de-este-cambio)) |
| [topic-key-warning](changes/topic-key-warning/proposal.md) | No sube: se cierra cuando aparezcan casos reales que verifiquen su criterio |

## Después del corte (no bloquean)

**Arquitectura, en este orden, porque cada paso abarata el siguiente:** store-boundary, memory-module-layout, core-use-cases, domain-types, storage-read-side, storage-write-side, y después search-index-ports y storage-ports, que tienen puerta: sin un candidato o una pregunta concreta, cierran sin código.

**Modelo y memoria del proceso:** tokenizer-footprint, embedder-bakeoff y model-reembed, que sólo sube si un modelo candidato pasa su umbral.

**Calidad de lo guardado:** memory-anchors, y una política de olvido para `memory`, que hoy no existe y todavía no tiene cambio.

## Cómo se mantiene

- Al cerrar una noche, lo hecho sale de acá y lo que se cruzó se anota en la exploración de su cambio.
- Un cambio nuevo entra en una etapa o abajo de todo, nunca en un lugar sin razón. Si no acerca la meta, va a «Después del corte».
- Los prompts de cada noche no viven acá: son notas personales que apuntan a estos cambios.
