//! What the model reads of each tool in a session whose language is Spanish:
//! the description of every tool and of every argument, which
//! [`super::server::ClawgMcp::with_language`] puts in place of the English
//! ones. Names, types and the values of an argument stay as they are; a test
//! keeps this table covering every tool and argument, and naming between
//! backticks what the English text names.

/// The tool, the argument (`None` for the tool itself) and its description.
pub const DESCRIPTIONS: &[(&str, Option<&str>, &str)] = &[
    (
        "delete_memory",
        None,
        "Borra una memoria guardada por id y tipo de fila. Primero toma el id de `search_memory`, \
         `list_memories` o `recall_context`: los ids son por tipo de fila, así que el mismo número existe en \
         las tres tablas. Una memoria marcada `by <agent>` la guardó otro agente, y borrarla la quita para todos \
         los agentes. Borrar la memoria actual de un `topic_key` vuelve actual a la versión que reemplazó. \
         `restore_memory` lo deshace.",
    ),
    (
        "delete_memory",
        Some("id"),
        "El id de la fila a borrar, como lo imprimen `search_memory`, `list_memories` o `recall_context`. Un hit \
         de `recall_context` sin tipo de fila delante de su tipo es una memoria.",
    ),
    (
        "delete_memory",
        Some("kind"),
        "A qué tabla pertenece el id: memory, episodic o procedural. Obligatorio, porque cada tabla numera sus \
         filas por separado: el id 7 existe en las tres y en cada una es otra cosa.",
    ),
    (
        "list_memories",
        None,
        "Recorre lo guardado, lo más nuevo primero, sin buscar. Úsala para mostrarle a la persona su inventario de \
         memorias o para encontrar un id que borrar. Lista cada memoria que puedes leer, tuyas y de otros agentes; \
         `agent` lista sólo las de un agente. Devuelve una línea por fila: id, tipo de fila, tipo, tier, \
         importancia, accesos, estado, `by <agent>` cuando la guardó otro agente, y un resumen recortado.",
    ),
    (
        "list_memories",
        Some("agent"),
        "Sólo las memorias que guardó este agente, como se nombra después de `by` en un listado o una búsqueda. \
         Sin él, lista las de cada agente que puedes leer.",
    ),
    (
        "list_memories",
        Some("kind"),
        "Qué tabla listar: memory, episodic o procedural. Sin él, lista las tres.",
    ),
    ("list_memories", Some("limit"), "Cuántas filas devolver, de 1 a 100. Por defecto 50."),
    (
        "list_memories",
        Some("memory_type"),
        "Sólo las filas de este tipo de memoria. Vale sólo para las filas `memory`.",
    ),
    (
        "list_memories",
        Some("offset"),
        "Cuántas filas saltear, para paginar una lista larga. Por defecto 0.",
    ),
    (
        "list_memories",
        Some("tier"),
        "Sólo las filas de este tier: 1 (caliente), 2 (tibio) o 3 (frío).",
    ),
    (
        "memory_status",
        None,
        "Dice si el daemon de memoria está corriendo, qué perfil y qué directorio de datos usa, y si su modelo de \
         embeddings terminó de cargar. Úsala cuando falla una herramienta de memoria y necesitas decirle a la \
         persona por qué.",
    ),
    (
        "read_memory",
        None,
        "Lee el texto entero de una memoria por id. Un texto largo se guarda en varias piezas, y la búsqueda muestra \
         una, marcada piece=K/N y cortada a 500 caracteres; pasa su id para obtener todas las piezas unidas en \
         orden. Devuelve qué pieza es el id, los ids del save (una pieza borrada aparece como faltante) y el texto.",
    ),
    (
        "read_memory",
        Some("id"),
        "El id de cualquier pieza del save, como lo imprimen `search_memory`, `list_memories`, `recall_context` o \
         `save_memory`.",
    ),
    (
        "read_memory",
        Some("kind"),
        "A qué tabla pertenece el id: memory, episodic o procedural. Por defecto memory, el único tipo de fila que \
         se guarda en piezas.",
    ),
    (
        "recall_context",
        None,
        "Obtén un bloque de contexto listo para leer sobre una consulta: las notas del workspace de la persona, las \
         memorias recientes y las de largo plazo, ya armadas y sin duplicados, con el contenido completo hasta un \
         presupuesto de bytes; lo que no entra se nombra por id para `read_memory`. Cuesta varias veces los tokens \
         de `search_memory`, así que úsala una vez al empezar una tarea amplia que necesita contexto, y \
         `search_memory` para preguntas puntuales.",
    ),
    (
        "recall_context",
        Some("budget"),
        "Cuántos bytes de contexto devolver como máximo, unos 4 por token. Por defecto 8000, a lo sumo 24000. Lo que \
         no entra se nombra por id al final.",
    ),
    (
        "recall_context",
        Some("project_path"),
        "Ruta absoluta del proyecto. Por defecto, el directorio en el que arrancó este puente.",
    ),
    (
        "recall_context",
        Some("query"),
        "Sobre qué pregunta la persona. El bloque devuelto es para leerlo como contexto antes de responder, no para \
         citárselo.",
    ),
    (
        "resolve_conflict",
        None,
        "Juzga un par de memorias que el detector encontró cercanas: uno que nombran `save_memory` o \
         `memory_status`, que juzgas tú, o uno bajo ACTIVE CONFLICTS en `recall_context`, una vez que la persona \
         dijo cuál vale. Una preferencia o una decisión sólo se reemplaza por palabra de la persona: júzgala \
         conflicts y pregunta, y después llama con supersedes, user_answered y su respuesta en reason. Devuelve el \
         estado de las dos memorias. Cualquiera de los lados puede ser de otro agente.",
    ),
    (
        "resolve_conflict",
        Some("keep"),
        "Un id de memoria del par, como lo muestran ACTIVE CONFLICTS, `save_memory` o `memory_status`. Con \
         supersedes es el que queda.",
    ),
    ("resolve_conflict", Some("other"), "El otro id de memoria del par."),
    (
        "resolve_conflict",
        Some("reason"),
        "Una frase sobre por qué; obligatoria para scoped, y con user_answered la respuesta de la persona.",
    ),
    (
        "resolve_conflict",
        Some("user_answered"),
        "True sólo cuando la persona dijo cuál vale, con su respuesta en reason. Reemplazar una user_preference o \
         una architectural_decision lo necesita.",
    ),
    (
        "resolve_conflict",
        Some("verdict"),
        "supersedes: `keep` reemplaza a `other`, que deja de recuperarse pero sigue listada y vuelve si el par se \
         juzga de nuevo. compatible: no se contradicen. scoped: valen las dos, cada una en su ámbito, dicho en \
         `reason`. conflicts: se contradicen, y la persona tiene que decir cuál vale.",
    ),
    (
        "restore_memory",
        None,
        "Deshace un `delete_memory`: trae de vuelta una memoria borrada, con el id y el tipo de fila con que se \
         borró, cuando la persona dice que el borrado fue un error. Se rechaza cuando el mismo texto se volvió a \
         guardar después, cuando una memoria más nueva tiene su `topic_key`, o cuando la memoria guarda un \
         secreto. Nombra las memorias guardadas mientras tanto que están cerca de ella, para juzgarlas con \
         `resolve_conflict`.",
    ),
    ("restore_memory", Some("id"), "El id que recibió `delete_memory`."),
    (
        "restore_memory",
        Some("kind"),
        "El tipo de fila que recibió `delete_memory`. Sólo se pueden restaurar filas memory.",
    ),
    (
        "save_memory",
        None,
        "Guarda algo que vale la pena recordar entre sesiones: un hecho sobre la persona o su proyecto, una \
         preferencia que dijo o una decisión que se tomó. Guarda una cosa por llamada, en pocas frases: un texto \
         largo se parte en piezas que la búsqueda encuentra de a una, y read_memory lo devuelve entero. Empieza con \
         una primera línea que nombre el tema, y después di qué, por qué y dónde, con las rutas e identificadores \
         exactos. Escríbelo para que tenga sentido dentro de meses, sin ninguna conversación alrededor. Devuelve el \
         id de cada pieza guardada, que aceptan update_memory y delete_memory, o por qué no se guardó nada.",
    ),
    (
        "save_memory",
        Some("content"),
        "La memoria a guardar, escrita como una afirmación que se entiende sola: se va a leer en una sesión futura \
         sin nada de esta conversación alrededor. Como máximo 64 KiB; parte un texto más largo en varios saves.",
    ),
    (
        "save_memory",
        Some("derived_from"),
        "Sólo para una conclusión: los ids de las memorias que generaliza, como los lista `synthesize`. Omítelo \
         para una memoria común.",
    ),
    (
        "save_memory",
        Some("importance"),
        "Cuánto importa, de 0 a 10. Los valores altos duran más y se recuperan antes. Por defecto 5.",
    ),
    (
        "save_memory",
        Some("memory_type"),
        "Qué tipo de memoria es, uno de los valores listados; cualquier otro se rechaza. Por defecto semantic_fact. \
         Una architectural_decision es una elección de diseño y su motivo; lo que se hizo, se encontró o se midió \
         es un semantic_fact, aun después de una decisión.",
    ),
    (
        "save_memory",
        Some("project_path"),
        "Ruta absoluta del proyecto al que pertenece esta memoria. Por defecto, el directorio en el que arrancó \
         este puente.",
    ),
    (
        "save_memory",
        Some("topic_key"),
        "Un nombre estable de aquello de lo que trata esta memoria, como `storage/engine`: segmentos en minúscula \
         de a-z, 0-9, '.', '_' o '-' unidos por '/', a lo sumo 80 caracteres. Guardar de nuevo bajo la misma clave \
         reemplaza la memoria guardada bajo ella, que queda en el historial. Omítelo para un tema nuevo.",
    ),
    (
        "save_memory",
        Some("user_stated"),
        "True sólo cuando el contenido es lo que dijo la persona: sus palabras o una instrucción explícita. Omítelo \
         para lo que inferiste, encontraste o concluiste; nunca con derived_from.",
    ),
    (
        "search_memory",
        None,
        "Encuentra memorias guardadas relacionadas con una consulta. Úsala primero, antes de responder algo que \
         dependa de una sesión anterior. La coincidencia es por significado, así que pasa una pregunta completa en \
         lenguaje natural, no palabras clave; encuentra memorias escritas con otras palabras, pero a menudo pierde \
         una escrita en otro idioma que la consulta. Devuelve una lista numerada de las memorias cercanas en \
         relevancia a la mejor (al menos dos), con sus ids, tipos de fila y puntajes; una lista corta significa que \
         el resto puntuó mucho más bajo, no que no haya nada más guardado. El contenido de más de 500 caracteres se \
         recorta. Un hit marcado piece=K/N es la pieza K, la que mejor coincide, de un texto largo guardado en N \
         piezas; read_memory con su id devuelve el texto entero.",
    ),
    (
        "search_memory",
        Some("all_projects"),
        "Busca en todos los proyectos en lugar de en uno. Úsalo cuando la persona pregunta por algo que no está \
         atado al código que tiene delante.",
    ),
    ("search_memory", Some("limit"), "Cuántas memorias devolver, de 1 a 50. Por defecto 10."),
    (
        "search_memory",
        Some("project_path"),
        "Ruta absoluta del proyecto en el que buscar. Por defecto, el directorio en el que arrancó este puente.",
    ),
    (
        "search_memory",
        Some("query"),
        "Qué buscar, en lenguaje natural. Las frases completas recuperan mejor que las palabras clave.",
    ),
    (
        "search_project",
        None,
        "Busca en la documentación del propio proyecto (README, docs, CLAUDE.md, notas de diseño, specs y \
         manifiestos) y en su código Rust, Python y TypeScript cómo funciona: su diseño, convenciones, comandos, \
         decisiones y dónde está implementado algo. Úsala antes de abrir archivos cuando la pregunta es cómo \
         funciona algo del proyecto o por qué es como es. La coincidencia es por significado y por término exacto, \
         así que pasa una pregunta o el nombre de la cosa. Devuelve pasajes numerados, cada uno bajo el archivo y \
         los encabezados o definiciones de los que sale; el índice sigue a los archivos en disco.",
    ),
    ("search_project", Some("limit"), "Cuántos pasajes devolver, de 1 a 20. Por defecto 5."),
    (
        "search_project",
        Some("project_path"),
        "Ruta absoluta del proyecto. Por defecto, el directorio en el que arrancó este puente. En los archivos de \
         otro proyecto sólo se busca una vez que la persona lo agregó con `clawg projects add`.",
    ),
    (
        "search_project",
        Some("query"),
        "Qué averiguar sobre el proyecto, como una pregunta en lenguaje natural o el nombre de algo que hay en él.",
    ),
    (
        "synthesize",
        None,
        "Revisa las memorias guardadas en busca de conclusiones: las memorias que todavía no cubre ninguna \
         conclusión, lo más nuevo primero, las conclusiones ya sacadas, las que perdieron una fuente borrada o \
         reemplazada después, y cómo guardar conclusiones nuevas o corregir las viejas. Una conclusión se guarda \
         con `save_memory` y `derived_from`; las memorias en las que descansa quedan como están. Úsala cuando \
         `recall_context` dice que hay memorias esperando o conclusiones para revisar, al final de una tarea larga, \
         o cuando la persona pregunta qué aprendiste. Llamarla marca las memorias como revisadas, aunque no \
         guardes nada.",
    ),
    (
        "synthesize",
        Some("all_projects"),
        "Revisa todos los proyectos a la vez, para conclusiones sobre la persona y no sobre un código.",
    ),
    ("synthesize", Some("limit"), "Cuántas memorias revisar, de 1 a 100. Por defecto 30."),
    (
        "synthesize",
        Some("project_path"),
        "Ruta absoluta del proyecto a revisar. Por defecto, el directorio en el que arrancó este puente.",
    ),
    (
        "update_memory",
        None,
        "Corrige la importancia, el tipo o el proyecto de una memoria guardada, por id y tipo de fila, cuando la \
         persona dice que importa más o menos de lo que se guardó, que es otro tipo de memoria, o que pertenece a \
         otro proyecto (to_project_path) o a ninguno (clear_project). Pasa sólo lo que cambia. Una memoria se mueve \
         con el resto de su save y de su historial de tema, y no se mueve adonde el mismo texto o la misma clave de \
         tema están vivos. Sacar una preferencia o una decisión de su tipo, o moverla, necesita user_answered, \
         puesto sólo cuando la persona lo dijo. Para cambiar lo que dice una memoria, guarda el texto nuevo y borra \
         el viejo. Devuelve la importancia, el tipo, el tier y, después de mover, el proyecto de la memoria, que ve \
         cada agente que la lee.",
    ),
    (
        "update_memory",
        Some("clear_project"),
        "True para sacar esta memoria de todos los proyectos, así la encuentran las búsquedas de cualquier \
         proyecto. No con to_project_path ni con to_project.",
    ),
    (
        "update_memory",
        Some("id"),
        "El id de la fila a cambiar, como lo imprimen `save_memory`, `search_memory`, `list_memories` o \
         `recall_context`. Un hit de `recall_context` sin tipo de fila delante de su tipo es una memoria.",
    ),
    (
        "update_memory",
        Some("importance"),
        "La importancia nueva, de 0 a 10. Omítela para dejarla como está.",
    ),
    (
        "update_memory",
        Some("kind"),
        "A qué tabla pertenece el id: memory, episodic o procedural.",
    ),
    (
        "update_memory",
        Some("memory_type"),
        "El tipo nuevo, sólo para kind memory, uno de los valores listados. Omítelo para dejar el tipo como está.",
    ),
    (
        "update_memory",
        Some("to_project_path"),
        "Un directorio del proyecto al que mover esta memoria, con el resto de su save y de su historial de tema, \
         cuando se guardó en el proyecto equivocado o en ninguno. Omítelo para dejar la memoria donde está.",
    ),
    (
        "update_memory",
        Some("user_answered"),
        "True sólo cuando la persona te lo pidió: sacar una user_preference o una architectural_decision de su \
         tipo, o moverla a otro proyecto, lo necesita.",
    ),
    (
        "answer_suggestion",
        None,
        "Registra la respuesta de la persona a una sugerencia de que este proyecto y otro pueden ser uno solo, \
         como la nombra `recall_context`. Pregúntale antes: merge pasa el otro proyecto y todo lo guardado en él \
         a este, y `clawg projects merge --undo` lo devuelve; apart los deja como dos proyectos y quita la \
         sugerencia. Necesita user_answered y sus palabras en reason.",
    ),
    (
        "answer_suggestion",
        Some("other"),
        "El id del otro proyecto, como lo nombra `recall_context`.",
    ),
    (
        "answer_suggestion",
        Some("answer"),
        "merge: el otro proyecto y todo lo guardado en él pasan a este. apart: siguen siendo dos proyectos, y no \
         se vuelven a sugerir.",
    ),
    ("answer_suggestion", Some("reason"), "La respuesta de la persona, con sus palabras."),
    (
        "answer_suggestion",
        Some("user_answered"),
        "True sólo cuando la persona respondió si son un solo proyecto.",
    ),
    (
        "answer_suggestion",
        Some("project"),
        "Este proyecto por su id o su nombre. Por defecto, el directorio en el que arrancó este puente.",
    ),
    (
        "save_memory",
        Some("project"),
        "El proyecto de esta memoria por su id o su nombre, para uno sin carpeta aquí, como una idea, o que no \
         es el de este directorio. Un nombre que ningún proyecto tiene ni se le parece crea el proyecto. No con \
         project_path.",
    ),
    (
        "save_memory",
        Some("new_project"),
        "True sólo cuando la persona dijo que un nombre parecido al de otros proyectos es un proyecto aparte: lo \
         crea aparte. Sólo con project.",
    ),
    ("search_memory", Some("project"), "Para uno sin carpeta aquí o que no es el de este directorio, el proyecto por su id o su nombre. No con \
         project_path."),
    ("recall_context", Some("project"), "Para uno sin carpeta aquí o que no es el de este directorio, el proyecto por su id o su nombre. No con \
         project_path."),
    ("search_project", Some("project"), "Para uno sin carpeta aquí o que no es el de este directorio, el proyecto por su id o su nombre. No con \
         project_path."),
    ("synthesize", Some("project"), "Para uno sin carpeta aquí o que no es el de este directorio, el proyecto por su id o su nombre. No con \
         project_path."),
    (
        "update_memory",
        Some("to_project"),
        "El proyecto al que mover esta memoria, por su id o su nombre, como to_project_path lo hace por un \
         directorio.",
    ),
];
