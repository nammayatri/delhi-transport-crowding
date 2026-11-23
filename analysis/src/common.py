"""Shared helpers for the validation scripts: paths, DuckDB access, chart style, Markdown
tables, and the gamma level rule used by the crowding model (ported so levels can be
re-derived for other expansion scenarios)."""

from __future__ import annotations

import json
import math
from decimal import Decimal
from pathlib import Path

import duckdb
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402
import polars as pl  # noqa: E402

# --------------------------------------------------------------------------------------
# Paths
# --------------------------------------------------------------------------------------

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "data" / "out"
RAW = ROOT / "data" / "raw"
TICKETS_CSV = RAW / "tickets" / "October-Delhi.csv"
GTFS = RAW / "gtfs" / "delhi-bus"
DOCS = ROOT / "docs"
FIG = DOCS / "figures"
RES = DOCS / "results"

RUNS = ["full", "train", "test", "sens_spread", "sens_cv", "sens_timetable", "sens_tickets", "sens_nocap", "sens_shrink"]

# Connaught Place, the reference point for inbound / outbound.
CP_LAT, CP_LON = 28.6315, 77.2167

# Peak windows used throughout (bin start minutes, half-open).
AM_PEAK = (8 * 60, 10 * 60)
PM_PEAK = (17 * 60 + 30, 20 * 60)

# Time bands for coverage tables: (label, start minute, end minute).
TIME_BANDS = [
    ("early 04:00-08:00", 240, 480),
    ("AM peak 08:00-10:00", 480, 600),
    ("midday 10:00-17:30", 600, 1050),
    ("PM peak 17:30-20:00", 1050, 1200),
    ("late 20:00-24:00", 1200, 1440),
]

LEVELS = ["seats", "standing", "crowded"]

SECTORS = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"]


def sector_of(lat: float, lon: float) -> str:
    """Compass sector (8-way) of a point as seen from Connaught Place."""
    dy = (lat - CP_LAT) * 111.2
    dx = (lon - CP_LON) * 111.2 * math.cos(math.radians(CP_LAT))
    ang = (math.degrees(math.atan2(dx, dy)) + 360) % 360
    return SECTORS[int((ang + 22.5) % 360 // 45)]


def bearing_words(lat: float, lon: float) -> str:
    """E.g. '16 km W of CP'."""
    dy = (lat - CP_LAT) * 111.2
    dx = (lon - CP_LON) * 111.2 * math.cos(math.radians(CP_LAT))
    return f"{math.hypot(dx, dy):.0f} km {sector_of(lat, lon)} of CP"


def ensure_dirs() -> None:
    FIG.mkdir(parents=True, exist_ok=True)
    RES.mkdir(parents=True, exist_ok=True)


def run_file(run: str, name: str) -> Path:
    return OUT / run / name


def load_json(run: str, name: str) -> dict:
    return json.loads(run_file(run, name).read_text())


# --------------------------------------------------------------------------------------
# DuckDB
# --------------------------------------------------------------------------------------


def connect(runs: list[str] | None = None) -> duckdb.DuckDBPyConnection:
    """In-memory DuckDB with views over the model outputs and GTFS stops.

    Views per run: seg_<run>, act_<run>, tix_<run>, trips_<run>, routes_<run>.
    Also: gtfs_stops(stop_id, stop_name, lat, lon), gtfs_routes, and the macro
    hav_km(lat1, lon1, lat2, lon2).
    """
    con = duckdb.connect()
    for run in runs or ["full"]:
        d = OUT / run
        con.sql(f"create or replace view seg_{run} as select * from read_parquet('{d / 'segment_profiles.parquet'}')")
        con.sql(f"create or replace view act_{run} as select * from read_parquet('{d / 'stop_activity.parquet'}')")
        con.sql(f"create or replace view tix_{run} as select * from read_parquet('{d / 'assigned_tickets.parquet'}')")
        con.sql(f"create or replace view trips_{run} as select * from read_parquet('{d / 'recon_trips.parquet'}')")
        con.sql(
            f"create or replace view routes_{run} as select * from "
            f"read_csv('{d / 'route_summary.csv'}', header=true, types={{'route_id': 'VARCHAR'}})"
        )
    con.sql(
        "create or replace macro hav_km(lat1, lon1, lat2, lon2) as "
        "2 * 6371.0 * asin(sqrt(pow(sin(radians(lat2 - lat1) / 2), 2) + "
        "cos(radians(lat1)) * cos(radians(lat2)) * pow(sin(radians(lon2 - lon1) / 2), 2)))"
    )
    stops = pl.read_csv(GTFS / "stops.txt", infer_schema=False).select(
        pl.col("stop_id"),
        pl.col("stop_name"),
        pl.col("stop_lat").cast(pl.Float64).alias("lat"),
        pl.col("stop_lon").cast(pl.Float64).alias("lon"),
    )
    con.register("gtfs_stops_df", stops.to_arrow())
    con.sql("create or replace table gtfs_stops as select * from gtfs_stops_df")
    routes = pl.read_csv(GTFS / "routes.txt", infer_schema=False)
    con.register("gtfs_routes_df", routes.to_arrow())
    con.sql("create or replace table gtfs_routes as select * from gtfs_routes_df")
    return con


def q(con: duckdb.DuckDBPyConnection, sql: str) -> pl.DataFrame:
    """Run a query and return a polars DataFrame."""
    return con.sql(sql).pl()


def scalar(con: duckdb.DuckDBPyConnection, sql: str):
    rows = con.sql(sql).fetchall()
    return rows[0][0] if rows else None


def segments_table(con: duckdb.DuckDBPyConnection, run: str, name: str, daytype: str | None = "weekday") -> None:
    """Materialise segment rows of one run with stop coordinates, straight-line segment
    length (the model's own distance definition) and direction relative to Connaught
    Place (inbound when the to-stop is closer to CP than the from-stop)."""
    where = f"where s.daytype = '{daytype}'" if daytype else ""
    con.sql(
        f"""
        create or replace table {name} as
        select s.*,
               a.lat as from_lat, a.lon as from_lon, b.lat as to_lat, b.lon as to_lon,
               hav_km(a.lat, a.lon, b.lat, b.lon) as seg_km,
               hav_km(a.lat, a.lon, {CP_LAT}, {CP_LON}) as from_cp_km,
               hav_km(b.lat, b.lon, {CP_LAT}, {CP_LON}) as to_cp_km,
               (a.lat + b.lat) / 2 as mid_lat, (a.lon + b.lon) / 2 as mid_lon,
               case when hav_km(b.lat, b.lon, {CP_LAT}, {CP_LON}) < hav_km(a.lat, a.lon, {CP_LAT}, {CP_LON})
                    then 'inbound' else 'outbound' end as direction
        from seg_{run} s
        join gtfs_stops a on a.stop_id = s.from_stop_id
        join gtfs_stops b on b.stop_id = s.to_stop_id
        {where}
        """
    )


# --------------------------------------------------------------------------------------
# Output helpers
# --------------------------------------------------------------------------------------


def save_json(name: str, obj: dict) -> Path:
    ensure_dirs()
    p = RES / f"{name}.json"
    p.write_text(json.dumps(obj, indent=2, default=_json_default))
    return p


def read_json(name: str) -> dict:
    return json.loads((RES / f"{name}.json").read_text())


def _json_default(o):
    if isinstance(o, Decimal):
        return float(o)
    if isinstance(o, (np.integer,)):
        return int(o)
    if isinstance(o, (np.floating,)):
        return float(o)
    if isinstance(o, np.ndarray):
        return o.tolist()
    raise TypeError(type(o))


def write_md(name: str, text: str) -> Path:
    ensure_dirs()
    p = RES / f"{name}.md"
    p.write_text(text.strip() + "\n")
    return p


def pct(x: float | None, digits: int = 1) -> str:
    if x is None or (isinstance(x, float) and math.isnan(x)):
        return "n/a"
    return f"{100 * x:.{digits}f}%"


def num(x: float | None, digits: int = 0) -> str:
    if x is None or (isinstance(x, float) and math.isnan(x)):
        return "n/a"
    return f"{x:,.{digits}f}"


def md_table(df: pl.DataFrame, fmt: dict[str, object] | None = None, align: dict[str, str] | None = None) -> str:
    """Markdown pipe table. `fmt` maps a column to a format spec ('{:.1f}') or callable."""
    fmt = fmt or {}
    align = align or {}
    cols = df.columns
    head = "| " + " | ".join(cols) + " |"
    seps = []
    for c in cols:
        a = align.get(c)
        if a is None:
            a = "r" if df.schema[c].is_numeric() else "l"
        seps.append("---:" if a == "r" else ":---")
    sep = "| " + " | ".join(seps) + " |"
    lines = [head, sep]
    for row in df.iter_rows(named=True):
        cells = []
        for c in cols:
            v = row[c]
            f = fmt.get(c)
            if v is None:
                s = ""
            elif callable(f):
                s = f(v)
            elif isinstance(f, str):
                s = f.format(v)
            elif isinstance(v, float):
                s = f"{v:,.2f}"
            elif isinstance(v, int) and not isinstance(v, bool):
                s = f"{v:,}"
            else:
                s = str(v)
            cells.append(s.replace("|", "/"))
        lines.append("| " + " | ".join(cells) + " |")
    return "\n".join(lines)


def hhmm(minute: int) -> str:
    return f"{minute // 60:02d}:{minute % 60:02d}"


def band_of(minute: int) -> str:
    for label, a, b in TIME_BANDS:
        if a <= minute < b:
            return label
    return "other"


def band_case_sql(col: str = "bin_minute") -> str:
    parts = " ".join(f"when {col} >= {a} and {col} < {b} then '{label}'" for label, a, b in TIME_BANDS)
    return f"case {parts} else 'other' end"


# --------------------------------------------------------------------------------------
# Chart style: one neutral palette, hairline axes, finding-first titles
# --------------------------------------------------------------------------------------

SURFACE = "#fcfcfb"
TEXT = "#0b0b0b"
TEXT2 = "#52514e"
MUTED = "#8a8984"
GRID = "#e6e5e1"
AXIS = "#c9c8c2"

BLUE = "#2a78d6"
ORANGE = "#eb6834"
AQUA = "#1baf7a"
YELLOW = "#eda100"
GREY = "#a3a29c"
CATEGORICAL = [BLUE, ORANGE, AQUA]

# Ordinal ramp (one hue, light to dark) for seats < standing < crowded.
LEVEL_COLORS = {"seats": "#86b6ef", "standing": "#2a78d6", "crowded": "#104281"}
# Sequential ramp for magnitudes.
SEQ_RAMP = ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"]

DAYTYPE_COLORS = {"weekday": BLUE, "saturday": ORANGE, "sunday": AQUA}


def setup_style() -> None:
    plt.rcParams.update(
        {
            "figure.facecolor": SURFACE,
            "axes.facecolor": SURFACE,
            "savefig.facecolor": SURFACE,
            "font.family": "sans-serif",
            "font.sans-serif": ["Helvetica Neue", "Helvetica", "Arial", "DejaVu Sans"],
            "font.size": 10,
            "axes.titlesize": 12,
            "axes.titleweight": "bold",
            "axes.titlelocation": "left",
            "axes.titlepad": 10,
            "axes.labelsize": 10,
            "axes.labelcolor": TEXT2,
            "axes.edgecolor": AXIS,
            "axes.linewidth": 0.8,
            "axes.spines.top": False,
            "axes.spines.right": False,
            "axes.grid": True,
            "axes.grid.axis": "y",
            "axes.axisbelow": True,
            "grid.color": GRID,
            "grid.linewidth": 0.6,
            "grid.linestyle": "-",
            "xtick.color": TEXT2,
            "ytick.color": TEXT2,
            "xtick.labelsize": 9,
            "ytick.labelsize": 9,
            "xtick.major.size": 3,
            "ytick.major.size": 0,
            "legend.frameon": False,
            "legend.fontsize": 9,
            "lines.linewidth": 2.0,
            "text.color": TEXT,
            "figure.dpi": 100,
        }
    )


def titles(ax, title: str, subtitle: str | None = None) -> None:
    """Finding as the title, metric definition as a muted subtitle."""
    if subtitle:
        ax.set_title(title, loc="left", pad=22)
        ax.text(0, 1.02, subtitle, transform=ax.transAxes, fontsize=9, color=TEXT2, va="bottom", ha="left")
    else:
        ax.set_title(title, loc="left")


def time_axis(ax, start: int = 240, end: int = 1440) -> None:
    ticks = list(range(start, end + 1, 120))
    ax.set_xticks(ticks)
    ax.set_xticklabels([hhmm(t) if t < 1440 else "24:00" for t in ticks])
    ax.set_xlim(start - 15, end + 15)
    ax.set_xlabel("time of day (30-min bin start)")


def save(fig, name: str) -> str:
    """Save a figure as PNG at 150 dpi and return its path relative to docs/results."""
    ensure_dirs()
    p = FIG / f"{name}.png"
    fig.savefig(p, dpi=150, bbox_inches="tight")
    plt.close(fig)
    return f"../figures/{name}.png"


# --------------------------------------------------------------------------------------
# The model's level rule, vectorised (ported from crowd-core levels.rs / stats.rs)
# --------------------------------------------------------------------------------------

_LANCZOS = np.array(
    [
        0.9999999999998099,
        676.5203681218851,
        -1259.1392167224028,
        771.3234287776531,
        -176.6150291621406,
        12.507343278686905,
        -0.13857109526572012,
        9.984369578019572e-6,
        1.5056327351493116e-7,
    ]
)


def ln_gamma(x: np.ndarray) -> np.ndarray:
    x = np.asarray(x, dtype=float)
    out = np.empty_like(x)
    small = x < 0.5
    big = ~small
    xb = x[big] - 1.0
    a = np.full_like(xb, _LANCZOS[0])
    t = xb + 7.5
    for i in range(1, 9):
        a += _LANCZOS[i] / (xb + i)
    out[big] = 0.5 * math.log(2 * math.pi) + (xb + 0.5) * np.log(t) - t + np.log(a)
    if np.count_nonzero(small):
        xs = x[small]
        out[small] = np.log(math.pi / np.sin(math.pi * xs)) - ln_gamma(1.0 - xs)
    return out


def gamma_p(a: np.ndarray, x: np.ndarray) -> np.ndarray:
    """Regularized lower incomplete gamma P(a, x), elementwise."""
    a, x = np.broadcast_arrays(np.asarray(a, dtype=float), np.asarray(x, dtype=float))
    a = a.copy()
    x = x.copy()
    out = np.zeros(a.shape)
    pos = x > 0
    ser = pos & (x < a + 1.0)
    cf = pos & ~ser
    if np.count_nonzero(ser):
        aa, xx = a[ser], x[ser]
        gln = ln_gamma(aa)
        ap = aa.copy()
        s = 1.0 / aa
        d = s.copy()
        for _ in range(500):
            ap += 1.0
            d *= xx / ap
            s += d
            if np.max(np.abs(d) / np.abs(s)) < 1e-12:
                break
        out[ser] = np.clip(np.exp(np.log(s) - xx + aa * np.log(xx) - gln), 0.0, 1.0)
    if np.count_nonzero(cf):
        aa, xx = a[cf], x[cf]
        gln = ln_gamma(aa)
        fpmin = 1e-300
        b = xx + 1.0 - aa
        c = np.full_like(b, 1.0 / fpmin)
        d = 1.0 / b
        h = d.copy()
        for i in range(1, 500):
            an = -i * (i - aa)
            b = b + 2.0
            d = an * d + b
            d = np.where(np.abs(d) < fpmin, fpmin, d)
            c = b + an / c
            c = np.where(np.abs(c) < fpmin, fpmin, c)
            d = 1.0 / d
            de = d * c
            h *= de
            if np.max(np.abs(de - 1.0)) < 1e-12:
                break
        out[cf] = np.clip(1.0 - np.exp(-xx + aa * np.log(xx) - gln) * h, 0.0, 1.0)
    return out


def level_probs(load, tickets, seats, comfortable, trip_cv: float):
    """Share of buses with seats / standing / crowded, and the median-bus level, exactly as
    the model computes them: gamma load with CV sqrt(trip_cv^2 + 1/max(tickets, 1))."""
    load = np.asarray(load, dtype=float)
    tickets = np.asarray(tickets, dtype=float)
    seats = np.asarray(seats, dtype=float)
    comfortable = np.asarray(comfortable, dtype=float)
    cv = np.maximum(np.sqrt(trip_cv**2 + 1.0 / np.maximum(tickets, 1.0)), 1e-3)
    shape = 1.0 / (cv * cv)
    scale = np.maximum(load, 1e-9) / shape
    f_seats = gamma_p(shape, seats / scale)
    f_comf = gamma_p(shape, comfortable / scale)
    p_seats = f_seats
    p_standing = np.maximum(f_comf - f_seats, 0.0)
    p_crowded = np.maximum(1.0 - f_comf, 0.0)
    zero = load <= 0
    p_seats = np.where(zero, 1.0, p_seats)
    p_standing = np.where(zero, 0.0, p_standing)
    p_crowded = np.where(zero, 0.0, p_crowded)
    level = np.where(p_crowded >= 0.5, 2, np.where(p_crowded + p_standing >= 0.5, 1, 0))
    return p_seats, p_standing, p_crowded, level


def level_names(codes: np.ndarray) -> np.ndarray:
    return np.array(LEVELS, dtype=object)[codes]


def spearman(a, b) -> float:
    """Spearman rank correlation (average ranks for ties)."""
    df = pl.DataFrame({"a": np.asarray(a, dtype=float), "b": np.asarray(b, dtype=float)})
    if df.height < 3:
        return float("nan")
    r = df.select(pl.corr("a", "b", method="spearman")).item()
    return float(r) if r is not None else float("nan")


def cohen_kappa(cm: np.ndarray) -> float:
    cm = np.asarray(cm, dtype=float)
    n = cm.sum()
    if n <= 0:
        return float("nan")
    po = np.trace(cm) / n
    pe = (cm.sum(axis=0) * cm.sum(axis=1)).sum() / (n * n)
    return float((po - pe) / (1 - pe)) if pe < 1 else float("nan")
