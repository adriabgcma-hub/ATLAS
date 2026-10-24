#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int main() {
    // Seteamos el UID a root explícitamente por seguridad
    setuid(0);
    setgid(0);
    
    // Ejecutamos ufw status
    // Usamos system() que es sencillo para este caso de lectura
    int ret = system("/usr/sbin/ufw status");
    
    return ret;
}
