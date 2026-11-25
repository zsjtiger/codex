-- A plugin added from the built-in marketplace remembers which catalog entry it came from.
ALTER TABLE plugins ADD COLUMN catalog_key text;
CREATE INDEX plugins_catalog ON plugins (catalog_key) WHERE catalog_key IS NOT NULL;
