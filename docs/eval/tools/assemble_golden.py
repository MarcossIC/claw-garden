#!/usr/bin/env python
"""
Ensambla el golden.json final: toma el draft minado, le inyecta las queries
escritas a mano, y le agrega los pares de contradiccion.

Los pares se arman en tres formas, y solo una de las tres necesita que alguien
invente texto:

  identical        — mitad texto real contra si mismo, mitad parafrasis del
                     mismo hecho. El `duplicate_count > 1` de engram parecia la
                     fuente natural, pero las 20 filas que lo llevan tienen
                     `content` vacio: son resumenes de sesion fallidos, no la
                     misma cosa dicha dos veces.
  no_contradiction — dos documentos del corpus que hablan de temas distintos.
                     Se eligen del mismo memory_type para que el par no sea
                     trivialmente separable por el tipo.
  contradiction    — la unica que se genera: una afirmacion real del corpus y
                     su negacion. Partir de texto real acota la invencion a la
                     negacion misma, que es verificable leyendo el par.
"""

import json
import re
import sqlite3
import sys

DRAFT = "golden.draft.json"
QUERIES = "queries.json"
OUT = "golden.json"
DB = "engram.db"

# Negaciones escritas contra afirmaciones reales del corpus. Cada una invierte
# UN hecho verificable del documento de origen — no agrega hechos nuevos, que
# es lo que haria al par ambiguo en vez de contradictorio.
CONTRADICTIONS = [
    ("c04", "El tipado genérico de Angular Forms (`FormGroup<T>`, `FormControl<T>`) es "
            "obligatorio y bloquea el commit si falta."),
    ("c24", "`CreditRequestWorkflowImpl` y `DocumentReviewWorkflowImpl` se registran con "
            "`providedIn: \"root\"`, igual que el resto de los workflows."),
    ("c25", "`AddressLookupController.doLookup` sigue escribiendo `countryId` en el "
            "`patchValue`, sobrescribiendo el valor inicial del control."),
    ("c30", "El botón secundario del footer de los modales de edición de cod SAP vinculado "
            "dice \"Cancelar\"."),
    ("c32", "Se extrajo un componente compartido `<app-person-name-inputs>` para los campos "
            "de nombre del board-modal."),
    ("c33", "The user chose ControlValueAccessor over ControlContainer for composing the "
            "child form with its parent."),
    ("c35", "El refactor se aplica primero en persona moral y después se replica en persona "
            "física, en el mismo PR."),
    ("c36", "Se respetó la regla de \"no scss\": el componente nuevo no lleva ninguna hoja "
            "de estilos propia."),
    ("c28", "El fallback de `getEstadoLabel` sigue devolviendo la cadena fija \"Incumplida\" "
            "cuando el estado no se reconoce."),
    ("c31", "El flujo optimista del modal de carga se aplicó también a sign-view, "
            "dcc-analysis-view y actions-view."),
]

# Pares no contradictorios: refs del corpus que hablan de cosas distintas dentro
# del mismo memory_type. Que compartan tipo es lo que hace util el par — un
# clasificador que solo mira el tipo no los puede separar.
NO_CONTRADICTION = [
    ("c01", "c07"), ("c05", "c12"), ("c13", "c18"), ("c16", "c21"),
    ("c22", "c29"), ("c23", "c27"), ("c34", "c31"), ("c37", "c42"),
    ("c38", "c40"), ("c43", "c46"),
]


def detect_lang(text: str) -> str:
    low = " " + text.lower() + " "
    es = sum(low.count(m) for m in (" que ", " de ", " el ", " la ", " se ", "ción", " para "))
    en = sum(low.count(m) for m in (" the ", " with ", " that ", " from ", " user "))
    return "es" if es >= en else "en"


# Reformulaciones del mismo hecho, escritas contra un documento real del
# corpus. Un `identical` que solo repite el string a caracter exacto es el caso
# facil; el que dice lo mismo con otras palabras es donde un detector de
# contradicciones mal calibrado dispara un falso positivo.
PARAPHRASES = [
    ("c30", "En el pie de esos modales de edición de cod SAP vinculado, el botón de la "
            "izquierda está rotulado \"Cerrar\"."),
    ("c34", "Como el usuario andaba con el tiempo justo, se acordó no sumar validaciones "
            "ni lógica extra: alcanza con guardar, recuperar el vigente y listar."),
    ("c35", "Primero se hace el refactor sobre persona física; recién cuando QA lo valide "
            "se replica en persona moral, en un PR aparte."),
    ("c32", "En el board-modal se decidió partir los campos inline, sin sacar un "
            "`<app-person-name-inputs>` compartido."),
]


def identical_pairs(by_ref: dict, exact_refs: list[str]) -> list[dict]:
    """
    Pares que dicen lo mismo y que el detector NO debe marcar: primero el texto
    real contra si mismo (el caso limite exacto), despues el mismo hecho
    reformulado (el caso que de verdad discrimina un umbral).
    """
    out = []
    for ref in exact_refs:
        body = by_ref[ref]["content"]
        out.append({"a": body, "b": body, "label": "identical", "lang": detect_lang(body)})
    for ref, restated in PARAPHRASES:
        out.append({
            "a": by_ref[ref]["content"],
            "b": restated,
            "label": "identical",
            "lang": detect_lang(restated),
        })
    return out


def main() -> int:
    draft = json.load(open(DRAFT, encoding="utf-8"))
    queries = json.load(open(QUERIES, encoding="utf-8"))
    by_ref = {r["ref"]: r for r in draft["corpus"]}

    # golden.json rechaza campos desconocidos: la procedencia y el texto de
    # ayuda del draft se quedan fuera a proposito.
    corpus = [
        {
            "ref": r["ref"],
            "content": r["content"],
            "memory_type": r["memory_type"],
            "importance": r["importance"],
            "lang": r["lang"],
        }
        for r in draft["corpus"]
    ]

    cases, missing = [], []
    for case in draft["draft_cases"]:
        ref = case["expected_refs"][0]
        query = queries.get(ref)
        if not query:
            missing.append(ref)
            continue
        cases.append({
            "query": query,
            "corpus_refs": case["corpus_refs"],
            "expected_refs": case["expected_refs"],
        })

    pairs = []
    for ref, negation in CONTRADICTIONS:
        pairs.append({
            "a": by_ref[ref]["content"],
            "b": negation,
            "label": "contradiction",
            "lang": detect_lang(negation),
        })
    for a_ref, b_ref in NO_CONTRADICTION:
        pairs.append({
            "a": by_ref[a_ref]["content"],
            "b": by_ref[b_ref]["content"],
            "label": "no_contradiction",
            "lang": detect_lang(by_ref[a_ref]["content"]),
        })
    pairs.extend(identical_pairs(by_ref, ["c04", "c19", "c25", "c40"]))

    golden = {"corpus": corpus, "retrieval_cases": cases, "contradiction_pairs": pairs}
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(golden, f, ensure_ascii=False, indent=2)

    label_counts = {}
    for p in pairs:
        label_counts[p["label"]] = label_counts.get(p["label"], 0) + 1

    print(f"corpus:              {len(corpus)}")
    print(f"retrieval_cases:     {len(cases)}")
    print(f"contradiction_pairs: {len(pairs)}  {label_counts}")
    if missing:
        print(f"\nSIN QUERY (caso omitido): {missing}")
    print(f"\nescrito: {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
