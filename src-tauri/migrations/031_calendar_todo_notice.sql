-- A todo the user made from a notice keeps a link back to it. The todo is the
-- user's own copy: a new revision of the notice leaves it unchanged, and
-- deleting the notice permanently only clears the link. A trigger clears it
-- instead of a foreign key so the column stays droppable.
ALTER TABLE calendar_todos ADD COLUMN document_id INTEGER;
CREATE INDEX calendar_todos_document ON calendar_todos(document_id) WHERE document_id IS NOT NULL;
CREATE TRIGGER calendar_todos_document_deleted AFTER DELETE ON documents BEGIN
  UPDATE calendar_todos SET document_id=NULL WHERE document_id=OLD.id;
END;
PRAGMA user_version=31;
