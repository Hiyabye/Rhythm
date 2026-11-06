-- Cybercampus (eClass) connector (#86). Course notices are archive documents of
-- the `cybercampus` source; these tables hold what is not a notice: the courses
-- of the synced term and their assignments and deadlines. The eClass login
-- lives only in the OS credential store, never here.
CREATE TABLE cybercampus_courses (
 course_key TEXT PRIMARY KEY,
 name TEXT NOT NULL,
 year INTEGER NOT NULL,
 term TEXT NOT NULL,
 synced_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
-- One row per eClass item with a deadline. `pending` marks items the site lists
-- as unfinished (its TODO list); `status` is the assignment page's own wording.
-- A sync replaces a course's rows with what eClass currently shows.
CREATE TABLE cybercampus_tasks (
 course_key TEXT NOT NULL REFERENCES cybercampus_courses(course_key) ON DELETE CASCADE,
 category TEXT NOT NULL,
 item_id TEXT NOT NULL,
 label TEXT NOT NULL,
 title TEXT NOT NULL,
 week INTEGER,
 opens_at TEXT,
 due_at TEXT,
 late_due_at TEXT,
 status TEXT,
 submitted_at TEXT,
 pending INTEGER NOT NULL DEFAULT 0 CHECK(pending IN (0,1)),
 updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(course_key,category,item_id)
);
CREATE INDEX cybercampus_tasks_due ON cybercampus_tasks(due_at);
PRAGMA user_version=18;
