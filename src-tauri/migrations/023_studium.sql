-- Only preferences are durable. Seat results and work stay in memory.
CREATE TABLE studium_selected_rooms(
  room_id TEXT PRIMARY KEY CHECK(room_id IN ('19','5','6','11','15','16','18'))
);
CREATE TABLE studium_preferences(
  id INTEGER PRIMARY KEY CHECK(id=1),
  view_room TEXT NOT NULL REFERENCES studium_selected_rooms(room_id)
);
INSERT INTO studium_selected_rooms(room_id) VALUES('5');
INSERT INTO studium_preferences(id,view_room) VALUES(1,'5');
INSERT INTO extension_preferences(extension_id,enabled) VALUES('studium',0)
ON CONFLICT(extension_id) DO NOTHING;
PRAGMA user_version=23;
