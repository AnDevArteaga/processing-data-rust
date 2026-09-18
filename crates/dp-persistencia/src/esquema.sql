CREATE TABLE IF NOT EXISTS organizaciones (
    id TEXT PRIMARY KEY,
    nombre TEXT NOT NULL,
    plan TEXT NOT NULL,
    creado_en TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    organizacion TEXT NOT NULL,
    nombre TEXT NOT NULL,
    prefijo TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE,
    revocada INTEGER NOT NULL DEFAULT 0,
    ultimo_uso TEXT,
    creada_en TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS saldos (
    organizacion TEXT PRIMARY KEY,
    disponible INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS reservas_credito (
    job_id TEXT PRIMARY KEY,
    organizacion TEXT NOT NULL,
    cantidad INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS movimientos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    organizacion TEXT NOT NULL,
    tipo TEXT NOT NULL,
    cantidad INTEGER NOT NULL,
    job_id TEXT,
    descripcion TEXT NOT NULL,
    creado_en TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS archivos (
    id TEXT PRIMARY KEY,
    organizacion TEXT NOT NULL,
    nombre_original TEXT NOT NULL,
    clave TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    tipo TEXT NOT NULL,
    estado TEXT NOT NULL,
    sha256 TEXT,
    creado_en TEXT NOT NULL,
    vence_en TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    organizacion TEXT NOT NULL,
    operacion TEXT NOT NULL,
    opciones TEXT NOT NULL,
    limites TEXT NOT NULL,
    entrada TEXT NOT NULL,
    salida TEXT,
    estado TEXT NOT NULL,
    progreso INTEGER NOT NULL,
    intentos INTEGER NOT NULL,
    max_intentos INTEGER NOT NULL,
    creditos_reservados INTEGER NOT NULL,
    creditos_cobrados INTEGER,
    resumen TEXT,
    error TEXT,
    creado_en TEXT NOT NULL,
    iniciado_en TEXT,
    terminado_en TEXT,
    reclamado_por TEXT,
    reclamado_hasta TEXT
);

CREATE INDEX IF NOT EXISTS jobs_cola ON jobs (estado, creado_en);
CREATE INDEX IF NOT EXISTS jobs_org ON jobs (organizacion, creado_en);
CREATE INDEX IF NOT EXISTS archivos_org ON archivos (organizacion);
CREATE INDEX IF NOT EXISTS movimientos_org ON movimientos (organizacion, id DESC);
