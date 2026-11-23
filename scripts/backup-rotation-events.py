#!/usr/bin/env python3
"""Crea/actualiza en Nextcloud los dos recordatorios alternos de rotación del
backup de atlas-hub (domingos 10:00, cada 2 semanas).

Uso (en atlas-hub, como root para poder leer el .env del relay):
    sudo python3 /opt/atlas/repo/scripts/backup-rotation-events.py

Idempotente: usa UIDs fijos; si el evento ya existe en el calendario, lo
reemplaza. Calendario destino: `Servidor` (se puede cambiar con
ATLAS_REMINDER_CALENDAR; debe existir ya en Nextcloud).
"""
import datetime
import os
import pathlib
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
ENV = pathlib.Path(os.environ.get("ATLAS_ENV", "/opt/atlas/repo/.env"))
sys.path.insert(0, str(REPO / "server"))

from atlas_caldav import CalDAVClient, build_vevent  # noqa: E402

CALENDAR_NAME = os.environ.get("ATLAS_REMINDER_CALENDAR", "Servidor")
RRULE = "FREQ=WEEKLY;INTERVAL=2"
DESCRIPTION = (
    "Tras la pasada de las 03:25: cambia la unidad ATLASBACKUP. Solo una "
    "conectada a la vez; la que estaba en el hub se va fuera de casa."
)
EVENTS = [
    ("atlas-backup-rotate-usb@atlas", "2026-11-22T10:00",
     "Backup Atlas: conecta el USB y llévate la SD"),
    ("atlas-backup-rotate-sd@atlas", "2026-11-29T10:00",
     "Backup Atlas: conecta la SD y llévate el USB"),
]


def load_env(path: pathlib.Path) -> dict[str, str]:
    data: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        data[key.strip()] = value.strip().strip('"').strip("'")
    return data


def with_rrule(ics: str) -> str:
    return ics.replace("END:VEVENT", f"RRULE:{RRULE}\r\nEND:VEVENT")


def verify(client: CalDAVClient, calendar: dict) -> int:
    """Lee los .ics guardados y muestra DTSTART/RRULE (sin modificar nada)."""
    missing = 0
    for uid, _, summary in EVENTS:
        url = client.find_event_url(calendar["id"], uid)
        if not url:
            print(f"FALTA: {summary}")
            missing += 1
            continue
        lines = [line for line in client.get_event(url).splitlines()
                 if line.startswith(("DTSTART", "RRULE"))]
        print(f"{summary}: {'; '.join(lines) or 'sin DTSTART/RRULE'}")
    return 1 if missing else 0


def main() -> int:
    env = load_env(ENV)
    missing = [k for k in ("CALDAV_USERNAME", "CALDAV_APP_PASSWORD") if k not in env]
    if missing:
        print(f"Falta {', '.join(missing)} en {ENV}", file=sys.stderr)
        return 1
    client = CalDAVClient(env["CALDAV_USERNAME"], env["CALDAV_APP_PASSWORD"],
                          base_url=env.get("CALDAV_URL"))
    calendars = client.discover()
    matches = [c for c in calendars
               if c["name"].strip().casefold() == CALENDAR_NAME.casefold()]
    if not matches:
        names = ", ".join(c["name"] for c in calendars)
        print(f"No existe el calendario {CALENDAR_NAME!r} (hay: {names})",
              file=sys.stderr)
        return 1
    calendar = matches[0]
    if "--verify" in sys.argv:
        return verify(client, calendar)
    for uid, start, summary in EVENTS:
        end_dt = datetime.datetime.strptime(start, "%Y-%m-%dT%H:%M")
        end = (end_dt + datetime.timedelta(minutes=30)).strftime("%Y-%m-%dT%H:%M")
        ics = with_rrule(build_vevent(uid, summary, start, end,
                                      description=DESCRIPTION))
        url = client.find_event_url(calendar["id"], uid)
        if url:
            client.put_event(url, ics)
            print(f"actualizado: {summary}")
        else:
            url = client.collection_url(calendar["id"]) + uid + ".ics"
            client.put_event(url, ics, new=True)
            print(f"creado: {summary}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
