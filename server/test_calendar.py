"""Pruebas offline de fechas de agenda (sin iCloud).

Cubren el incidente del 2026-11-06: la herramienta devolvía el instante sin
zona ni día de la semana y el modelo lo descolocaba. Las fechas esperadas están
calculadas a mano.
"""

from __future__ import annotations

import datetime
import unittest
from zoneinfo import ZoneInfo

import atlas_caldav
import atlas_relay


class ParseVeventTests(unittest.TestCase):
    def test_conserva_el_tzid_de_dtstart_y_dtend(self):
        parsed = atlas_caldav.parse_vevent(
            "UID:synthetic-a\n"
            "SUMMARY:Clase\n"
            "DTSTART;TZID=Europe/Madrid:20261109T103000\n"
            "DTEND;TZID=Europe/Madrid:20261109T113000\n"
        )
        self.assertEqual(parsed["start"], "20261109T103000")
        self.assertEqual(parsed["start_params"], ";TZID=Europe/Madrid")
        self.assertEqual(parsed["end_params"], ";TZID=Europe/Madrid")

    def test_marca_los_eventos_de_todo_el_dia(self):
        parsed = atlas_caldav.parse_vevent(
            "UID:synthetic-b\n"
            "SUMMARY:Entrega\n"
            "DTSTART;VALUE=DATE:20261112\n"
        )
        self.assertEqual(parsed["start"], "20261112")
        self.assertEqual(parsed["start_params"], ";VALUE=DATE")


class ResolveDatetimeTests(unittest.TestCase):
    def test_con_tzid_se_convierte_a_la_zona_local(self):
        moment, all_day = atlas_caldav.resolve_datetime(
            "20261109T103000", ";TZID=Europe/Madrid", "Europe/Madrid"
        )
        self.assertFalse(all_day)
        self.assertEqual(moment, datetime.datetime(2026, 11, 9, 10, 30,
                                                  tzinfo=ZoneInfo("Europe/Madrid")))

    def test_utc_z_se_convierte_a_la_zona_local(self):
        moment, all_day = atlas_caldav.resolve_datetime(
            "20261109T093000Z", "", "Europe/Madrid"
        )
        self.assertFalse(all_day)
        self.assertEqual(moment, datetime.datetime(2026, 11, 9, 10, 30,
                                                  tzinfo=ZoneInfo("Europe/Madrid")))

    def test_todo_el_dia_devuelve_fecha_sin_hora(self):
        moment, all_day = atlas_caldav.resolve_datetime(
            "20261109", ";VALUE=DATE", "Europe/Madrid"
        )
        self.assertTrue(all_day)
        self.assertEqual(moment, datetime.date(2026, 11, 9))


class WeekBoundsTests(unittest.TestCase):
    def test_el_viernes_6_de_noviembre_la_semana_siguiente_empieza_el_9(self):
        now = datetime.datetime(2026, 11, 6, 12, 0, tzinfo=ZoneInfo("Europe/Madrid"))
        start, end = atlas_relay.week_bounds(now)
        self.assertEqual(start.date(), datetime.date(2026, 11, 9))
        self.assertEqual((end - datetime.timedelta(days=1)).date(),
                         datetime.date(2026, 11, 15))

    def test_el_domingo_8_la_semana_siguiente_sigue_siendo_el_9(self):
        now = datetime.datetime(2026, 11, 8, 20, 0, tzinfo=ZoneInfo("Europe/Madrid"))
        start, _ = atlas_relay.week_bounds(now)
        self.assertEqual(start.date(), datetime.date(2026, 11, 9))


class EventsWithinTests(unittest.TestCase):
    def test_filtra_solo_la_semana_siguiente(self):
        events = [
            {"start": "20261106T120000", "start_params": ";TZID=Europe/Madrid",
             "summary": "hoy"},
            {"start": "20261109T103000", "start_params": ";TZID=Europe/Madrid",
             "summary": "lunes"},
            {"start": "20261112", "start_params": ";VALUE=DATE",
             "summary": "entrega"},
            {"start": "20261115T235900", "start_params": ";TZID=Europe/Madrid",
             "summary": "domingo"},
            {"start": "20261116T000000", "start_params": ";TZID=Europe/Madrid",
             "summary": "siguiente"},
        ]
        start, end = atlas_relay.week_bounds(
            datetime.datetime(2026, 11, 6, 12, 0, tzinfo=ZoneInfo("Europe/Madrid")))
        picked = [e["summary"] for e in
                  atlas_relay.events_within(events, start, end, "Europe/Madrid")]
        self.assertEqual(picked, ["lunes", "entrega", "domingo"])


class FormatEventLabelTests(unittest.TestCase):
    def test_etiqueta_con_dia_semanal_hora_y_zona(self):
        event = atlas_caldav.parse_vevent(
            "UID:synthetic-c\n"
            "SUMMARY:C3 MUS\n"
            "DTSTART;TZID=Europe/Madrid:20261109T103000\n"
        )
        self.assertEqual(
            atlas_relay.format_event_label(event, "Europe/Madrid"),
            "lunes 9 nov 10:30",
        )

    def test_etiqueta_de_todo_el_dia(self):
        event = atlas_caldav.parse_vevent(
            "UID:synthetic-d\n"
            "SUMMARY:Entrega\n"
            "DTSTART;VALUE=DATE:20261112\n"
        )
        self.assertEqual(
            atlas_relay.format_event_label(event, "Europe/Madrid"),
            "jueves 12 nov (todo el día)",
        )


if __name__ == "__main__":
    unittest.main()
