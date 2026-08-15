#!/usr/bin/env python
"""
Extractor de corpus candidato para el golden set de rag-eval, desde engram.db.

Lee una COPIA read-only de ~/.engram/engram.db y emite documentos de memoria
realistas: texto que el sistema RAG de claw-garden podria haber almacenado de
verdad, porque es exactamente el tipo de cosa que almacena.

Politica de privacidad: DESCARTAR, no limpiar. Hay ~26 candidatos por cada
documento que el golden set necesita, asi que un documento con un dato
sensible que no se sabe mapear se tira entero. Intentar sanearlo es apostar a
que la regex cubrio todos los casos; tirarlo no apuesta nada.

No decide queries ni distractores: eso es la etapa siguiente (hard negative
mining con el embedder real). Aca solo sale el material crudo, ya mapeado al
esquema que golden.rs valida.
"""

import json
import re
import sqlite3
import sys
from collections import Counter, defaultdict

DB = "engram.db"
OUT = "corpus_candidates.json"

TYPE_MAP = {
    "architecture": "architectural_decision",
    "decision": "architectural_decision",
    "bugfix": "tool_error",
    "bug": "tool_error",
    "session_summary": "session_summary",
    "pattern": "semantic_fact",
    "discovery": "semantic_fact",
    "config": "semantic_fact",
    "convention": "semantic_fact",
    "learning": "semantic_fact",
    "preference": "user_preference",
}

# Sustituciones estables: el RAG mide estructura semantica, no le importa si
# el servicio se llama portas-api o svc-alpha. Un placeholder consistente
# preserva la relacion "estos dos documentos hablan del mismo sistema", que es
# justamente la senal que hace duros a los distractores.
ANON = {
    "portas-api": "svc-alpha", "portas-web": "web-alpha", "portas": "alpha",
    "dfsm-dashboard": "dash-beta", "dfsm": "beta",
    "distribuidores-web": "web-gamma", "distribuidores-db-scripts": "db-gamma",
    "distribuidores": "gamma",
    "credit-request-service": "svc-delta", "credit-request": "req-delta",
    "quotation-tool-service": "svc-epsilon", "quotation": "epsilon",
    "interting-chat-ia": "chat-zeta", "interting": "zeta",
    "pdf-service": "svc-pdf",
    "DaimlerCS": "VendorSans", "Daimler": "Vendor", "daimler": "vendor",
    "DAIMLER": "VENDOR",
    "Freeway": "Upstream", "freeway": "upstream", "FREEWAY": "UPSTREAM",
    "DocuSign": "ESignProvider", "docusign": "esignprovider",
    "Docusign": "ESignProvider", "DOCUSIGN": "ESIGNPROVIDER",
    "Metamap": "IdVerifier", "METAMAP": "IDVERIFIER", "metamap": "idverifier",
    "MetaMap": "IdVerifier",
    "Marcos": "el usuario", "marcos": "el usuario", "MarcossIC": "el usuario",
}

# Si algo de esto sobrevive a ANON, el documento se descarta. No se intenta
# reemplazar: un patron sensible que llego hasta aca es justamente el caso que
# las sustituciones no previeron.
SENSITIVE = [
    (re.compile(r"[\w.\-]+@[\w.\-]+\.\w+"), "email"),
    (re.compile(r"https?://(?!localhost|127\.0\.0\.1)[\w.\-]+"), "url_externa"),
    (re.compile(r"\b\d{1,3}(?:\.\d{1,3}){3}\b"), "ip"),
    (re.compile(r"#[0-9a-fA-F]{6}\b"), "color_hex"),
    (re.compile(r"[A-Za-z]:[\\/]"), "ruta_absoluta"),
    (re.compile(r"\b\d{7,}\b"), "id_numerico_largo"),
]

# Un secreto real mezcla letras y digitos sin separadores. Exigir >=6 digitos
# es lo que distingue un hash de un identificador camelCase largo: sin ese
# piso, `cleanupStaleBoardOnShareholderRfcChange` (39 chars) se descarta como
# si fuera una credencial.
TOKEN_LIKE = re.compile(r"\b(?=(?:[A-Za-z]*\d){6,})[A-Za-z0-9]{32,}\b")

# Terminos que identifican el negocio de un cliente. Cada uno se verifico
# contra la DB antes de entrar: `mora` va con \b porque sin el matchea
# "persona moral" y "demorada" y tiraba 66 documentos sanos; `carpeta`,
# `credito` y `cotizacion` salieron de la lista por genericos.
DENY = [
    "legajo", "expediente", "concesionario", "cuit", "curp", "rfc",
    "poliza", "póliza", "siniestro", "aval",
]
DENY_WORD = [re.compile(r"\bmora\b", re.I)]

ES_MARKERS = (
    " que ", " para ", " porque ", " usuario ", " con ", " los ", " las ",
    " del ", " una ", " este ", " esta ", "ción", " cuando ", " sin ", " ya ",
)
EN_MARKERS = (
    " the ", " with ", " because ", " when ", " which ", " this ", " that ",
    " from ", " into ", " does ", " should ", " were ", " been ",
)


def anonymize(text: str) -> str:
    for real, fake in ANON.items():
        text = text.replace(real, fake)
    text = re.sub(r"/(?:home|Users)/[\w.\-]+/", "~/", text)
    return text


def sensitivity_reason(text: str) -> str | None:
    """Primer motivo por el que este documento no puede viajar a un repo."""
    for pattern, label in SENSITIVE:
        if pattern.search(text):
            return label
    if TOKEN_LIKE.search(text):
        return "token_largo"
    low = text.lower()
    for term in DENY:
        if term in low:
            return f"deny:{term}"
    for pattern in DENY_WORD:
        if pattern.search(text):
            return f"deny:{pattern.pattern}"
    return None


def detect_lang(text: str) -> str:
    low = " " + text.lower() + " "
    es = sum(low.count(m) for m in ES_MARKERS)
    en = sum(low.count(m) for m in EN_MARKERS)
    return "es" if es >= en else "en"


def extract_what(content: str) -> str | None:
    """
    Nucleo afirmativo de una observacion con formato **What**/**Why**/...

    El bloque What es una afirmacion autocontenida — que es exactamente la
    forma de un documento de memoria real. Why/Where/Learned son metadata de
    proceso y arrastran rutas de archivo que solo agregan ruido lexico.
    """
    m = re.search(r"\*\*What\*\*:\s*(.+?)(?=\n\s*\*\*(?:Why|Where|Learned)\*\*:|\Z)",
                  content, re.S)
    if not m:
        return None
    body = re.sub(r"```.*?```", " ", m.group(1), flags=re.S)
    return re.sub(r"\s+", " ", body).strip() or None


def extract_summary(content: str) -> str | None:
    """
    Los session_summary no usan **What**: usan secciones markdown (## Goal,
    ## Discoveries, ...). El resumen honesto de una sesion es su Goal, que es
    la frase que un `session_summary` real llevaria como contenido.
    """
    m = re.search(r"##\s*(?:Goal|Objetivo)\s*\n(.+?)(?=\n##|\Z)", content, re.S)
    if not m:
        return None
    body = re.sub(r"```.*?```", " ", m.group(1), flags=re.S)
    body = re.sub(r"[*_`#]", "", body)
    return re.sub(r"\s+", " ", body).strip() or None


def extract_preferences(content: str) -> list[str]:
    """
    Preferencias del usuario escondidas en los session_summary.

    El tipo `preference` de engram tiene solo 13 filas en toda la DB, pero la
    seccion `## Instructions` de cada resumen de sesion registra exactamente lo
    mismo — que hay que responder en espanol, que no se buildea, que no se
    tocan los templates mobile — y hay 176 bullets de esos. Cada bullet ya es
    una preferencia autocontenida en lenguaje natural, que es la forma que
    tiene un `user_preference` real.
    """
    m = re.search(r"##\s*(?:Instructions|Instrucciones)\s*\n(.+?)(?=\n##|\Z)",
                  content, re.S)
    if not m:
        return []
    out = []
    for bullet in re.findall(r"^\s*[-*]\s+(.+)$", m.group(1), re.M):
        text = re.sub(r"[*_`]", "", bullet)
        out.append(re.sub(r"\s+", " ", text).strip())
    return out


def extract_code(content: str) -> str | None:
    m = re.search(r"```[\w+]*\n(.+?)```", content, re.S)
    if not m:
        return None
    code = m.group(1).strip()
    return code if 40 <= len(code) <= 400 else None


def importance_of(mtype: str, revision_count: int, duplicate_count: int) -> float:
    """
    Importancia derivada de senales reales, no inventada: el tipo fija la base
    y la evidencia de que el tema volvio (revisiones, duplicados) la sube.
    Acotada a [3.0, 9.5], el rango que ya usa el golden set actual.
    """
    base = {
        "architectural_decision": 7.0,
        "user_preference": 6.0,
        "semantic_fact": 5.0,
        "session_summary": 5.0,
        "tool_error": 4.0,
        "code_block": 3.5,
        "chat_message": 3.0,
    }[mtype]
    bump = min(2.5, 0.5 * (revision_count - 1) + 0.5 * (duplicate_count - 1))
    return round(min(9.5, base + bump), 1)


def main() -> int:
    con = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    con.row_factory = sqlite3.Row

    rows = con.execute("""
        SELECT id, type, title, content, project, topic_key,
               revision_count, duplicate_count
        FROM observations
        WHERE deleted_at IS NULL AND LENGTH(content) > 200
        ORDER BY id
    """).fetchall()

    out, seen = [], set()
    dropped = Counter()

    def emit(src_id, project, body, mtype, rev, dup, lang=None, min_len=120):
        if not body or not (min_len <= len(body) <= 700):
            dropped["largo_fuera_de_rango"] += 1
            return
        body = anonymize(body)
        reason = sensitivity_reason(body)
        if reason:
            dropped[reason] += 1
            return
        key = body[:120].lower()
        if key in seen:
            dropped["duplicado"] += 1
            return
        seen.add(key)
        out.append({
            "src_id": src_id,
            "src_project": project,
            "content": body,
            "memory_type": mtype,
            "importance": importance_of(mtype, rev, dup),
            "lang": lang or detect_lang(body),
        })

    for r in rows:
        mtype = TYPE_MAP.get(r["type"])
        if not mtype:
            dropped["tipo_sin_mapeo"] += 1
            continue
        body = extract_summary(r["content"]) if mtype == "session_summary" \
            else extract_what(r["content"])
        if body is None:
            dropped["sin_bloque_extraible"] += 1
            continue
        emit(r["id"], r["project"], body, mtype,
             r["revision_count"], r["duplicate_count"])

    # user_preference tambien vive en los bullets de `## Instructions`, no solo
    # en el tipo `preference` (que tiene 13 filas en toda la DB).
    for r in rows:
        if r["type"] != "session_summary":
            continue
        for i, pref in enumerate(extract_preferences(r["content"])):
            emit(f"i{r['id']}-{i}", r["project"], pref, "user_preference",
                 1, 1, min_len=80)

    # code_block sale de las fences, que los extractores de arriba descartan.
    for r in rows:
        code = extract_code(r["content"])
        if code:
            emit(f"k{r['id']}", r["project"], code, "code_block",
                 r["revision_count"], 1, lang="en")

    # chat_message sale de los prompts reales del usuario. El piso de largo
    # baja porque solo hay 8 prompts en la DB y un pedido corto sigue siendo
    # un mensaje de chat legitimo.
    for r in con.execute("SELECT id, content, project FROM user_prompts"):
        emit(f"p{r['id']}", r["project"],
             re.sub(r"\s+", " ", r["content"]).strip(), "chat_message",
             1, 1, min_len=60)

    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, indent=2)

    by_type = defaultdict(Counter)
    for d in out:
        by_type[d["memory_type"]][d["lang"]] += 1

    print(f"candidatos aceptados: {len(out)}")
    print(f"descartados:          {sum(dropped.values())}\n")
    print(f"{'memory_type':26} {'total':>6} {'es':>5} {'en':>5}")
    for t in sorted(by_type, key=lambda k: -sum(by_type[k].values())):
        c = by_type[t]
        print(f"  {t:24} {sum(c.values()):6} {c['es']:5} {c['en']:5}")
    print("\nmotivos de descarte:")
    for reason, n in dropped.most_common():
        print(f"  {reason:26} {n:6}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
