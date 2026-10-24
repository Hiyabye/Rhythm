CREATE TABLE settings (
 id INTEGER PRIMARY KEY CHECK(id=1),
 theme TEXT NOT NULL DEFAULT 'system' CHECK(theme IN ('system','light','dark')),
 energy_profile TEXT NOT NULL DEFAULT 'balanced' CHECK(energy_profile IN ('eco','balanced','performance')),
 interests TEXT NOT NULL DEFAULT ''
);
INSERT INTO settings(id) VALUES(1);
CREATE TABLE extension_preferences (
 extension_id TEXT PRIMARY KEY NOT NULL,
 enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1))
);
PRAGMA user_version=1;
