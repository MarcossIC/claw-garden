-- V22: the language each memory row is written in.
--
-- 'en' or 'es' as the detector reads the stored (masked) text, 'und' when it
-- cannot tell, NULL until the row is classified: every row of a store from
-- before this migration starts NULL, and the maintenance pass fills it in
-- without touching anything else. While a store holds both languages, a
-- search in one counts the live rows of the other, which a query in the first
-- is likely to miss.
--
-- No trigger of `memory` reads this column, so classifying a row rewrites no
-- text index. Undoing it is
--   DROP INDEX idx_memory_lang; DROP INDEX idx_memory_lang_unset;
--   ALTER TABLE memory DROP COLUMN lang;
-- since SQLite refuses to drop an indexed column.
ALTER TABLE memory ADD COLUMN lang TEXT CHECK (lang IN ('en', 'es', 'und'));

-- The count of live rows in one language.
CREATE INDEX IF NOT EXISTS idx_memory_lang ON memory (lang)
    WHERE deleted_at IS NULL AND status IN ('active', 'disputed');

-- The rows still to classify; empty once the maintenance pass has run.
CREATE INDEX IF NOT EXISTS idx_memory_lang_unset ON memory (id) WHERE lang IS NULL;
