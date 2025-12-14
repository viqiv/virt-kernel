# virt-kernel
kernel for virt aarch64 board on qemu.

```
mkdir ~/init
cd ~/init
cat <<EOF > init.c
#include <stdio.h>
int main() {
  char buf[20] = {0};
  printf("Enter your name: \n");
  fflush(stdout);
  scanf("%s", buf);
  printf("Hello %s\n", buf);
  return 0;
}
EOF

aarch64-linux-gnu-gcc init.c -o init -static

# Edit run.sh
#    [...]
#    -fsdev local,id=fs0,path=~/init,security_model=mapped-xattr \
#    [...]

# for some reasons debug is chewing too much stack
cargo run --target aarch64-unknown-none-softfloat --release

```
