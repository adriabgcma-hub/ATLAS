"""CALDAV_URL configurable: base por defecto iCloud y base efectiva del cliente."""

import os
import unittest
from unittest import mock

import atlas_caldav

NEXTCLOUD = "https://atlas-hub:8080/remote.php/dav"


def _fake_request_factory(recorded: list[tuple[str, str]], root: str):
    """PROPFIND mínimos y válidos para que `discover()` llegue al listado."""

    def fake(url, method, username, password, body,
             depth=None, timeout=20, retries=3):
        recorded.append((url, method))
        principal = root + "/principals/u/"
        home = root + "/calendars/u/"
        if body and "current-user-principal" in body:
            return f"""<?xml version="1.0" encoding="utf-8" ?>
                <D:multistatus xmlns:D="DAV:"><D:response>
                <D:href>/</D:href><D:propstat><D:prop>
                <D:current-user-principal><D:href>{principal}</D:href>
                </D:current-user-principal></D:prop></D:propstat>
                </D:response></D:multistatus>""".encode("utf-8")
        if body and "calendar-home-set" in body:
            return f"""<?xml version="1.0" encoding="utf-8" ?>
                <D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
                <D:response><D:href>{principal}</D:href><D:propstat><D:prop>
                <C:calendar-home-set><D:href>{home}</D:href></C:calendar-home-set>
                </D:prop></D:propstat></D:response></D:multistatus>""".encode("utf-8")
        return f"""<?xml version="1.0" encoding="utf-8" ?>
            <D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
            <D:response><D:href>{home}</D:href><D:propstat><D:prop>
            <D:displayname>u</D:displayname><D:resourcetype><D:collection/></D:resourcetype>
            </D:prop></D:propstat></D:response>
            <D:response><D:href>{home}personal/</D:href><D:propstat><D:prop>
            <D:displayname>Personal</D:displayname>
            <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
            </D:prop></D:propstat></D:response></D:multistatus>""".encode("utf-8")

    return fake


class CalDavUrlTest(unittest.TestCase):
    def test_por_defecto_icloud(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(atlas_caldav.caldav_base(),
                             "https://caldav.icloud.com")

    def test_variable_vacia_usa_icloud(self):
        with mock.patch.dict(os.environ, {"CALDAV_URL": "   "}, clear=True):
            self.assertEqual(atlas_caldav.caldav_base(),
                             "https://caldav.icloud.com")

    def test_con_variable_nextcloud(self):
        with mock.patch.dict(os.environ, {"CALDAV_URL": NEXTCLOUD}, clear=True):
            self.assertEqual(atlas_caldav.caldav_base(), NEXTCLOUD)

    def test_el_cliente_usa_la_base_configurada(self):
        recorded: list[tuple[str, str]] = []
        with mock.patch.dict(os.environ, {"CALDAV_URL": NEXTCLOUD}, clear=True), \
                mock.patch.object(atlas_caldav, "_request",
                                  side_effect=_fake_request_factory(recorded,
                                                                    NEXTCLOUD)):
            client = atlas_caldav.CalDAVClient("u", "p")
            calendars = client.discover()
        self.assertEqual(client.base, NEXTCLOUD)
        self.assertEqual(client.well_known, NEXTCLOUD + "/.well-known/caldav")
        self.assertTrue(recorded, "discover() no hizo ninguna petición")
        self.assertEqual(recorded[0][0], NEXTCLOUD + "/")
        for url, _method in recorded:
            self.assertTrue(url.startswith(NEXTCLOUD), url)
            self.assertNotIn("icloud.com", url)
        self.assertEqual([c["name"] for c in calendars], ["Personal"])

    def test_el_cliente_sin_variable_usa_icloud_como_antes(self):
        recorded: list[tuple[str, str]] = []
        with mock.patch.dict(os.environ, {}, clear=True), \
                mock.patch.object(atlas_caldav, "_request",
                                  side_effect=_fake_request_factory(
                                      recorded, "https://caldav.icloud.com")):
            client = atlas_caldav.CalDAVClient("u", "p")
            client.discover()
        self.assertEqual(client.base, "https://caldav.icloud.com")
        self.assertEqual(client.well_known,
                         "https://caldav.icloud.com/.well-known/caldav")
        self.assertEqual(recorded[0][0], "https://caldav.icloud.com/")

    def test_base_explicita_con_barra_final_no_duplica(self):
        client = atlas_caldav.CalDAVClient("u", "p", base_url=NEXTCLOUD + "/")
        self.assertEqual(client.base, NEXTCLOUD)
        self.assertEqual(client.well_known, NEXTCLOUD + "/.well-known/caldav")


if __name__ == "__main__":
    unittest.main()
