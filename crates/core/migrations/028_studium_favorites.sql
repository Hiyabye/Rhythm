-- Favorites are user preferences, independent of the selected lookup rooms.
-- Never persist availability, source responses or seat-check history here.
CREATE TABLE studium_favorite_seats(
  room_id TEXT NOT NULL CHECK(room_id IN ('19','5','6','11','15','16','18')),
  seat_number INTEGER NOT NULL CHECK(
    typeof(seat_number)='integer' AND seat_number BETWEEN 1 AND CASE room_id
      WHEN '19' THEN 113 WHEN '5' THEN 102 WHEN '6' THEN 311
      WHEN '11' THEN 101 WHEN '15' THEN 207 WHEN '16' THEN 92
      WHEN '18' THEN 174 END
  ),
  PRIMARY KEY(room_id,seat_number)
);
PRAGMA user_version=28;
