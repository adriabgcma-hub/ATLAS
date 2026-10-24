#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int main() {
    // Forzamos el UID a root
    setuid(0);
    setgid(0);
    
    // Ejecutamos ufw status directamente sin pasar por un shell
    // Esto evita que el shell suelte los privilegios de root (S-bit)
    char *args[] = {"/usr/sbin/ufw", "status", NULL};
    execv("/usr/sbin/ufw", args);
    
    // Si execv falla, devolvemos error
    return 1;
}
