"""Cliente CalDAV mínimo para iCloud (solo stdlib, sin dependencias).

Lee calendarios y eventos próximos. Sin escritura en v1 (llegará con las
tools). Autenticación Basic con Apple ID + contraseña de app.

Flujo: principal → calendar-home-set → colecciones VCALENDAR → REPORT con
filtro de tiempo (+ expansión de recurrencias del lado servidor).
"""

from __future__ import annotations

import base64
import datetime
import time
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET

DAV = "{DAV:}"
CALDAV = "{urn:ietf:params:xml:ns:caldav}"

def _absolute(base: str, href: str) -> str:
    """El href CalDAV puede ser relativo (/…) o absoluto (shard de Apple)."""
    if href.startswith("http://") or href.startswith("https://"):
        return href
    return base + href


BASE = "https://caldav.icloud.com"

USER_AGENT = "AtlasRelay/3"


class CalDAVError(Exception):
    """Fallo contra el servidor CalDAV (mensaje seguro, sin secretos)."""


def _auth_header(username: str, password: str) -> str:
    token = base64.b64encode(f"{username}:{password}".encode("utf-8")).decode("ascii")
    return f"Basic {token}"


def _request(url: str, method: str, username: str, password: str,
             body: str | None, depth: str | None = None,
             timeout: int = 20) -> bytes:
    data = body.encode("utf-8") if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", _auth_header(username, password))
    req.add_header("User-Agent", USER_AGENT)
    if data is not None:
        req.add_header("Content-Type", "application/xml; charset=utf-8")
        req.add_header("Content-Length", str(len(data)))
    if depth is not None:
        req.add_header("Depth", depth)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return response.read()
    except urllib.error.HTTPError as exc:
        if exc.code == 401:
            raise CalDAVError("credenciales rechazadas") from exc
        if exc.code == 403:
            # Apple limita (throttle) ante ráfagas; no son las credenciales.
            raise CalDAVError("limitado por iCloud, reintentar") from exc
        if exc.code == 404:
            raise CalDAVError("recurso no encontrado") from exc
        raise CalDAVError(f"http_{exc.code}") from exc
    except Exception as exc:
        raise CalDAVError(f"red ({type(exc).__name__})") from exc


def _hrefs(multistatus: bytes) -> list[str]:
    items = []
    try:
        root = ET.fromstring(multistatus)
    except ET.ParseError:
        return items
    for response in root.findall(f"{DAV}response"):
        href = response.findtext(f"{DAV}href")
        if href:
            items.append(href)
    return items


def _prop(xml: bytes, *path: str) -> str | None:
    try:
        root = ET.fromstring(xml)
    except ET.ParseError:
        return None
    node = root
    for tag in path:
        node = node.find(f".//{tag}")
        if node is None:
            return None
    return node.text


class CalDAVClient:
    """Conexión autenticada a los calendarios iCloud."""

    WELL_KNOWN = "https://caldav.icloud.com/.well-known/caldav"

    def __init__(self, username: str, password: str) -> None:
        self.username = username
        self.password = password
        self.home_set = ""
        self.calendars: list[dict] = []

    def _call(self, url: str, method: str, body: str | None = None,
              depth: str | None = None) -> bytes:
        return _request(url, method, self.username, self.password, body, depth)

    def discover(self) -> list[dict]:
        """Autentica y lista calendarios. Devuelve [{id, name}]."""
        # 1. Principal del usuario actual.
        root = self._call("https://caldav.icloud.com/", "PROPFIND",
                          f"""<?xml version="1.0" encoding="utf-8" ?>
                          <D:propfind xmlns:D="DAV:"><D:prop>
                          <D:current-user-principal /></D:prop></D:propfind>""",
                          depth="0")
        principal = _prop(root, f"{DAV}current-user-principal", f"{DAV}href")
        if not principal:
            raise CalDAVError("sin principal")
        principal_url = _absolute(BASE, principal)
        # 2. Casa de calendarios.
        home_xml = self._call(principal_url, "PROPFIND",
                              f"""<?xml version="1.0" encoding="utf-8" ?>
                              <D:propfind xmlns:D="DAV:"
                              xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop>
                              <C:calendar-home-set /></D:prop></D:propfind>""",
                              depth="0")
        home = _prop(home_xml, f"{CALDAV}calendar-home-set", f"{DAV}href")
        if not home:
            raise CalDAVError("sin calendar-home-set")
        self.home_set = _absolute(BASE, home).rstrip("/") + "/"
        # 3. Colecciones: quedarse con las que son calendarios (VCALENDAR).
        listing = self._call(self.home_set, "PROPFIND",
                             f"""<?xml version="1.0" encoding="utf-8" ?>
                             <D:propfind xmlns:D="DAV:"
                             xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop>
                             <D:displayname /><D:resourcetype /></D:prop></D:propfind>""",
                             depth="1")
        calendars = []
        try:
            tree = ET.fromstring(listing)
        except ET.ParseError as exc:
            raise CalDAVError("listado ilegible") from exc
        for response in tree.findall(f"{DAV}response"):
            href = response.findtext(f"{DAV}href") or ""
            if href.rstrip("/") == self.home_set.rstrip("/"):
                continue  # la propia casa, no un calendario
            supported = response.find(f".//{CALDAV}supported-calendar-component-set")
            types = set()
            if supported is not None:
                for comp in supported.findall(f"{CALDAV}comp"):
                    if comp.get("name"):
                        types.add(comp.get("name"))
            is_calendar = response.find(f".//{CALDAV}calendar") is not None
            if not is_calendar and "VEVENT" not in types and types:
                continue
            name = response.findtext(f".//{DAV}displayname") or href.strip("/").split("/")[-1]
            # Colecciones de sistema de Apple (scheduling): sin eventos propios.
            if name.strip().casefold() in ("inbox", "outbox", "notification"):
                continue
            calendars.append({"id": href, "name": name})
        self.calendars = calendars
        return calendars

    def upcoming(self, days: int = 7, calendar_id: str | None = None) -> list[dict]:
        """Eventos desde ahora hasta `days` días (recurrencias expandidas)."""
        if not self.home_set:
            self.discover()
        now = datetime.datetime.now(datetime.timezone.utc)
        end = now + datetime.timedelta(days=max(1, min(days, 31)))
        stamp = "%Y%m%dT%H%M%SZ"
        targets = [c for c in self.calendars
                   if calendar_id is None or c["id"] == calendar_id]
        events = []
        for index, calendar in enumerate(targets):
            # iCloud limita ráfagas: pausa entre REPORTs.
            if index > 0:
                time.sleep(1.5)
            url = _absolute(BASE, calendar["id"])
            report = self._call(url, "REPORT",
                                f"""<?xml version="1.0" encoding="utf-8" ?>
                                <C:calendar-query xmlns:D="DAV:"
                                xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop>
                                <D:getetag /><C:calendar-data>
                                <C:expand start="{now.strftime(stamp)}"
                                end="{end.strftime(stamp)}" />
                                </C:calendar-data></D:prop>
                                <C:filter><C:comp-filter name="VCALENDAR">
                                <C:comp-filter name="VEVENT">
                                <C:time-range start="{now.strftime(stamp)}"
                                end="{end.strftime(stamp)}" />
                                </C:comp-filter></C:comp-filter></C:filter>
                                </C:calendar-query>""",
                                depth="1")
            for vevent in parse_icalendar(report):
                vevent["calendar"] = calendar["name"]
                events.append(vevent)
        events.sort(key=lambda e: e.get("start") or "")
        return events

    def collection_url(self, calendar_id: str) -> str:
        """URL de la colección (redescubre si el id no está en caché)."""
        matches = [c for c in self.calendars if c["id"] == calendar_id]
        if not matches:
            self.discover()
            matches = [c for c in self.calendars if c["id"] == calendar_id]
        if not matches:
            raise CalDAVError("calendario desconocido")
        return _absolute(BASE, matches[0]["id"]).rstrip("/") + "/"

    def _calendar_url(self, calendar_id: str) -> str:
        return self.collection_url(calendar_id)

    def find_event_url(self, calendar_id: str, uid: str) -> str | None:
        """URL del evento con ese UID (match exacto en cliente: el prop-filter
        de Apple devuelve 412)."""
        base = self._calendar_url(calendar_id)
        report = self._call(base, "REPORT",
                            """<?xml version="1.0" encoding="utf-8" ?>
                            <C:calendar-query xmlns:D="DAV:"
                            xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop>
                            <D:getetag /><C:calendar-data /></D:prop>
                            <C:filter><C:comp-filter name="VCALENDAR">
                            <C:comp-filter name="VEVENT" />
                            </C:comp-filter></C:filter></C:calendar-query>""",
                            depth="1")
        try:
            root = ET.fromstring(report)
        except ET.ParseError as exc:
            raise CalDAVError("búsqueda ilegible") from exc
        for response in root.findall(f"{DAV}response"):
            href = response.findtext(f"{DAV}href")
            data = response.findtext(f".//{CALDAV}calendar-data")
            if not href or not data:
                continue
            for block in split_vevents(data):
                parsed = parse_vevent(block)
                if parsed and parsed.get("uid") == uid:
                    return _absolute(BASE, href)
        return None

    def get_event(self, url: str) -> str:
        req = urllib.request.Request(url, method="GET")
        req.add_header("Authorization", _auth_header(self.username, self.password))
        req.add_header("User-Agent", USER_AGENT)
        try:
            with urllib.request.urlopen(req, timeout=20) as response:
                return response.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as exc:
            raise CalDAVError(f"http_{exc.code}") from exc
        except Exception as exc:
            raise CalDAVError(f"red ({type(exc).__name__})") from exc

    def put_event(self, url: str, ics: str, new: bool = False) -> None:
        data = ics.encode("utf-8")
        req = urllib.request.Request(url, data=data, method="PUT")
        req.add_header("Authorization", _auth_header(self.username, self.password))
        req.add_header("User-Agent", USER_AGENT)
        req.add_header("Content-Type", "text/calendar; charset=utf-8")
        req.add_header("Content-Length", str(len(data)))
        if new:
            req.add_header("If-None-Match", "*")
        try:
            with urllib.request.urlopen(req, timeout=20):
                pass
        except urllib.error.HTTPError as exc:
            if exc.code == 412:
                raise CalDAVError("ya existe") from exc
            raise CalDAVError(f"http_{exc.code}") from exc
        except Exception as exc:
            raise CalDAVError(f"red ({type(exc).__name__})") from exc

    def delete_event(self, url: str) -> None:
        req = urllib.request.Request(url, method="DELETE")
        req.add_header("Authorization", _auth_header(self.username, self.password))
        req.add_header("User-Agent", USER_AGENT)
        try:
            with urllib.request.urlopen(req, timeout=20):
                pass
        except urllib.error.HTTPError as exc:
            raise CalDAVError(f"http_{exc.code}") from exc
        except Exception as exc:
            raise CalDAVError(f"red ({type(exc).__name__})") from exc


def parse_icalendar(multistatus: bytes) -> list[dict]:
    """Extrae VEVENTs de una respuesta REPORT (tolerante, solo campos útiles)."""
    try:
        root = ET.fromstring(multistatus)
    except ET.ParseError:
        return []
    events = []
    for response in root.findall(f"{DAV}response"):
        data = response.findtext(f".//{CALDAV}calendar-data")
        if not data:
            continue
        for vevent in split_vevents(data):
            parsed = parse_vevent(vevent)
            if parsed:
                events.append(parsed)
    return events


def split_vevents(ical: str) -> list[str]:
    # Desplegado de líneas (espacio inicial = continuación).
    unfolded = []
    for line in ical.replace("\r\n", "\n").split("\n"):
        if line[:1] in (" ", "\t") and unfolded:
            unfolded[-1] += line[1:]
        else:
            unfolded.append(line)
    blocks, current = [], []
    inside = False
    for line in unfolded:
        if line == "BEGIN:VEVENT":
            inside, current = True, []
        elif line == "END:VEVENT" and inside:
            blocks.append(current)
            inside = False
        elif inside:
            current.append(line)
    return ["\n".join(block) for block in blocks]


def parse_vevent(block: str) -> dict | None:
    fields: dict[str, str] = {}
    for line in block.split("\n"):
        if ":" not in line:
            continue
        name, _, value = line.partition(":")
        key = name.split(";")[0]
        if key in ("UID", "SUMMARY", "DTSTART", "DTEND", "STATUS", "LOCATION"):
            fields[key.lower()] = value.strip()
    if "uid" not in fields:
        return None
    return {
        "uid": fields.get("uid", ""),
        "summary": fields.get("summary", "(sin título)"),
        "start": fields.get("dtstart", ""),
        "end": fields.get("dtend", ""),
        "status": fields.get("status", ""),
        "location": fields.get("location", ""),
    }


def _escape(text: str) -> str:
    """Escape TEXT según RFC 5545 (el modelo manda texto libre)."""
    return (text.replace("\\", "\\\\").replace("\n", "\\n")
                 .replace(";", "\\;").replace(",", "\\,"))
def parse_vevent_with_url(calendar_data: str, url: str) -> dict | None:
    """Un VEVENT suelto (GET directo) + su URL para actualizar/borrar."""
    for block in split_vevents(calendar_data):
        parsed = parse_vevent(block)
        if parsed:
            parsed["url"] = url
            return parsed
    return None


def _dt_line(prop: str, value: str, timezone: str) -> str:
    """Línea DTSTART/DTEND desde `YYYY-MM-DD` (todo el día) o `YYYY-MM-DDTHH:MM`
    (se interpreta en `timezone` y se guarda en UTC)."""
    import datetime as dt
    import zoneinfo
    if "T" not in value:
        return f"{prop};VALUE=DATE:{value.replace('-', '')}"
    moment = dt.datetime.strptime(value[:16], "%Y-%m-%dT%H:%M")
    moment = moment.replace(tzinfo=zoneinfo.ZoneInfo(timezone))
    return f"{prop}:" + moment.astimezone(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def patch_vevent(ics: str, summary: str | None = None, start: str | None = None,
                 end: str | None = None, description: str | None = None,
                 location: str | None = None,
                 timezone: str = "Europe/Madrid") -> str | None:
    """Parche textual de un VEVENT (conserva UID y el resto intacto).
    Devuelve el ics modificado o None si no hay VEVENT."""
    lines = []
    for raw in ics.replace("\r\n", "\n").split("\n"):
        lines.append(raw)
    try:
        begin = next(i for i, line in enumerate(lines) if line == "BEGIN:VEVENT")
        finish = next(i for i, line in enumerate(lines) if line == "END:VEVENT")
    except StopIteration:
        return None
    # Trabaja desplegado dentro del bloque.
    block: list[str] = []
    for line in lines[begin + 1:finish]:
        if line[:1] in (" ", "\t") and block:
            block[-1] += line[1:]
        else:
            block.append(line)
    patched: list[str] = []
    seen = set()
    replacements = {
        "SUMMARY": summary, "DESCRIPTION": description, "LOCATION": location,
    }
    if start:
        replacements["DTSTART"] = _dt_line("DTSTART", start, timezone)
        raw_dtstart = True
    else:
        raw_dtstart = False
    if end:
        replacements["DTEND"] = _dt_line("DTEND", end, timezone)
        raw_dtend = True
    else:
        raw_dtend = False
    for line in block:
        key = line.split(";")[0].split(":")[0]
        if key in ("SUMMARY", "DESCRIPTION", "LOCATION") and replacements.get(key):
            if key in seen:
                continue
            seen.add(key)
            patched.append(f"{key}:{_escape(replacements[key])}")
        elif key == "DTSTART" and raw_dtstart:
            if key in seen:
                continue
            seen.add(key)
            patched.append(replacements["DTSTART"])
        elif key == "DTEND" and raw_dtend:
            if key in seen:
                continue
            seen.add(key)
            patched.append(replacements["DTEND"])
        else:
            patched.append(line)
    return "\r\n".join(lines[:begin + 1] + patched + lines[finish:])


def build_vevent(uid: str, summary: str, start: str, end: str | None = None,
                 description: str = "", location: str = "",
                 timezone: str = "Europe/Madrid") -> str:
    """Construye un VEVENT. `start`/`end`: `YYYY-MM-DD` (todo el día) o
    `YYYY-MM-DDTHH:MM` (se interpreta en `timezone` y se guarda en UTC)."""
    import datetime as dt
    import zoneinfo
    all_day = "T" not in start
    if all_day:
        dtstart = f"DTSTART;VALUE=DATE:{start.replace('-', '')}"
        dtend = ""
        if end:
            dtend = f"\r\nDTEND;VALUE=DATE:{end.replace('-', '')}"
    else:
        zone = zoneinfo.ZoneInfo(timezone)
        start_dt = dt.datetime.strptime(start[:16], "%Y-%m-%dT%H:%M").replace(tzinfo=zone)
        start_utc = start_dt.astimezone(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        dtstart = f"DTSTART:{start_utc}"
        dtend = ""
        if end and "T" in end:
            end_dt = dt.datetime.strptime(end[:16], "%Y-%m-%dT%H:%M").replace(tzinfo=zone)
            dtend = "\r\nDTEND:" + end_dt.astimezone(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    lines = [
        "BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Atlas//Atlas//ES",
        "BEGIN:VEVENT", f"UID:{uid}", f"DTSTAMP:{stamp}", dtstart + dtend,
        f"SUMMARY:{_escape(summary)}",
    ]
    if description:
        lines.append(f"DESCRIPTION:{_escape(description)}")
    if location:
        lines.append(f"LOCATION:{_escape(location)}")
    lines.append("END:VEVENT")
    lines.append("END:VCALENDAR")
    # Plegado a 75 octetos por línea (RFC 5545).
    raw = "\r\n".join(lines)
    folded = []
    for line in raw.split("\r\n"):
        encoded = line.encode("utf-8")
        while len(encoded) > 75:
            cut = 75
            while cut > 0 and (encoded[cut] & 0xC0) == 0x80:
                cut -= 1
            folded.append(encoded[:cut].decode("utf-8"))
            encoded = b" " + encoded[cut:]
        folded.append(encoded.decode("utf-8"))
    return "\r\n".join(folded)
