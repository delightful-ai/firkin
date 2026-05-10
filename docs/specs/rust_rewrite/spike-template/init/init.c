// Tiny static-musl init for the spike initramfs. Writes a sentinel to
// console, then powers off. Kernel picks this up automatically as /init
// from the cpio archive.
//
// Edit as needed for your spike. If you need to keep the guest running,
// replace the reboot() call with a pause() loop.

#include <fcntl.h>
#include <linux/reboot.h>
#include <stddef.h>
#include <sys/reboot.h>
#include <sys/syscall.h>
#include <unistd.h>

static const char HELLO[] = "SPIKE: hello from init\n";
static const char BYE[]   = "SPIKE: powering off\n";

static void write_str(int fd, const char *s) {
    const char *p = s;
    while (*p) p++;
    (void)write(fd, s, (size_t)(p - s));
}

int main(void) {
    write_str(1, HELLO);
    write_str(1, BYE);

    sync();
    syscall(SYS_reboot, LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2,
            LINUX_REBOOT_CMD_POWER_OFF, NULL);

    for (;;) pause();
}
