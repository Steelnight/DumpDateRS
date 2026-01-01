-- Users table
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY, -- Telegram Chat ID
    location_id TEXT NOT NULL,
    notify_time TEXT NOT NULL DEFAULT '18:00',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Index on users(location_id) for faster reverse lookups from events and distinct location queries
CREATE INDEX IF NOT EXISTS idx_users_location_id ON users(location_id);

-- Subscriptions table
CREATE TABLE IF NOT EXISTS subscriptions (
    user_id INTEGER NOT NULL,
    waste_type TEXT NOT NULL,
    PRIMARY KEY (user_id, waste_type),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

-- Pickup events table
CREATE TABLE IF NOT EXISTS pickup_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    location_id TEXT NOT NULL,
    date DATE NOT NULL,
    waste_type TEXT NOT NULL,
    UNIQUE(location_id, date, waste_type)
);
