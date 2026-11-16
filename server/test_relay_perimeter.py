import ipaddress
import unittest

from atlas_relay import client_allowed


class TestClientAllowed(unittest.TestCase):
    def test_cliente_lan_permitido(self):
        nets = [ipaddress.ip_network("192.168.100.0/24")]
        self.assertTrue(client_allowed("192.168.100.26", nets))

    def test_cliente_fuera_rechazado(self):
        nets = [ipaddress.ip_network("192.168.100.0/24")]
        self.assertFalse(client_allowed("8.8.8.8", nets))

    def test_cliente_tailnet_permitido_con_red_extra(self):
        nets = [ipaddress.ip_network("192.168.100.0/24"),
                ipaddress.ip_network("100.64.0.0/10")]
        self.assertTrue(client_allowed("100.64.0.5", nets))

    def test_ip_invalida_rechazada(self):
        nets = [ipaddress.ip_network("192.168.100.0/24")]
        self.assertFalse(client_allowed("no-es-ip", nets))


if __name__ == "__main__":
    unittest.main()