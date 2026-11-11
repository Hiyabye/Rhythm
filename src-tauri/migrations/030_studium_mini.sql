-- The mini window is always opened explicitly and always-on-top is session-only.
-- A missing row means native centering/default size, not a restored open window.
CREATE TABLE studium_mini_geometry(
  id INTEGER PRIMARY KEY CHECK(id=1),
  x INTEGER NOT NULL CHECK(typeof(x)='integer' AND x BETWEEN -100000 AND 100000),
  y INTEGER NOT NULL CHECK(typeof(y)='integer' AND y BETWEEN -100000 AND 100000),
  width INTEGER NOT NULL CHECK(typeof(width)='integer' AND width BETWEEN 300 AND 1200),
  height INTEGER NOT NULL CHECK(typeof(height)='integer' AND height BETWEEN 260 AND 1400)
);
PRAGMA user_version=30;
