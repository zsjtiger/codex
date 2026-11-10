-- Text read from a project file, kept so chats can use it and search can find it.
ALTER TABLE attachments ADD COLUMN extracted_text text;
