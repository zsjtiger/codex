-- A turn whose reply is saved as a document of its chat when it completes: the summary turns.
ALTER TABLE turns ADD COLUMN save_as_document boolean NOT NULL DEFAULT false;
