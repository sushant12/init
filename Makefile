ARCH := $(shell uname -m)
BIN_DIR := bin
WGET_QUIET := -q
RELEASE_URL := https://github.com/firecracker-microvm/firecracker/releases
CARGO_TARGET = x86_64-unknown-linux-musl
SOCKET_PATH ?= /tmp/firecracker.socket
VSOCK_PATH ?= /tmp/firecracker-vsock.sock
$(shell mkdir -p $(BIN_DIR))

.PHONY: all clean firecracker kernel rootfs build tmpinit run 

all: kernel firecracker rootfs build tmpinit run

kernel: $(BIN_DIR)/vmlinux

firecracker: $(BIN_DIR)/firecracker

rootfs: $(BIN_DIR)/rootfs.ext4

build:
	@echo "Building init..."
	cargo build --release --target $(CARGO_TARGET)
	@echo "Init binary built at target/$(CARGO_TARGET)/release/init"

tmpinit:
	@echo "Setting up tmpinit device..."
	fallocate -l 64M $(BIN_DIR)/tmpinit
	mkfs.ext2 $(BIN_DIR)/tmpinit
	mkdir -p $(BIN_DIR)/initmount
	-sudo umount $(BIN_DIR)/initmount
	sudo mount -o loop,noatime $(BIN_DIR)/tmpinit $(BIN_DIR)/initmount
	sudo mkdir -p $(BIN_DIR)/initmount/firestarter
	sudo cp target/$(CARGO_TARGET)/release/init $(BIN_DIR)/initmount/firestarter/init
	sudo cp run.json $(BIN_DIR)/initmount/firestarter/run.json
	sudo umount $(BIN_DIR)/initmount
	sudo rm -rf $(BIN_DIR)/initmount

run:
	@echo "Running Firecracker..."
	sudo rm -f $(SOCKET_PATH)
	sudo rm -f $(VSOCK_PATH)
	sudo $(BIN_DIR)/firecracker --api-sock $(SOCKET_PATH) --config-file config.json

$(BIN_DIR)/vmlinux:
	@echo "Fetching latest kernel version..."
	$(eval LATEST := $(shell wget "http://spec.ccfc.min.s3.amazonaws.com/?prefix=firecracker-ci/v1.11/$(ARCH)/vmlinux-5.10&list-type=2" -O - 2>/dev/null | grep -oP "(?<=<Key>)(firecracker-ci/v1.11/$(ARCH)/vmlinux-5\.10\.[0-9]{1,3})(?=</Key>)"))
	@echo "Downloading kernel binary..."
	wget $(WGET_QUIET) "https://s3.amazonaws.com/spec.ccfc.min/$${LATEST}" -O $(BIN_DIR)/vmlinux
	@echo "Kernel binary downloaded to $(BIN_DIR)/vmlinux"

$(BIN_DIR)/firecracker:
	@echo "Fetching latest Firecracker release..."
	$(eval FC_LATEST := $(shell basename $$(curl -fsSLI -o /dev/null -w %{url_effective} $(RELEASE_URL)/latest)))
	@echo "Downloading Firecracker $(FC_LATEST)..."
	curl -L $(RELEASE_URL)/download/$(FC_LATEST)/firecracker-$(FC_LATEST)-$(ARCH).tgz \
		| tar -xz
	mv release-$(FC_LATEST)-$(ARCH)/firecracker-$(FC_LATEST)-$(ARCH) $(BIN_DIR)/firecracker
	rm -rf release-$(FC_LATEST)-$(ARCH)
	@echo "Firecracker binary downloaded to $(BIN_DIR)/firecracker"

$(BIN_DIR)/rootfs.ext4:
	@echo "Creating rootfs from Docker image $(DOCKER_IMAGE)..."
	@# Create a temporary directory for the rootfs
	mkdir -p $(BIN_DIR)/rootfs
	@# Export the Docker image contents
	docker export $$(docker create $(DOCKER_IMAGE)) | tar -C $(BIN_DIR)/rootfs -xf -
	@# Create an empty ext4 file system
	dd if=/dev/zero of=$(BIN_DIR)/rootfs.ext4 bs=1M count=$(ROOTFS_SIZE)
	mkfs.ext4 $(BIN_DIR)/rootfs.ext4
	@# Mount the file system and copy contents
	mkdir -p $(BIN_DIR)/mnt
	sudo mount $(BIN_DIR)/rootfs.ext4 $(BIN_DIR)/mnt
	sudo cp -r $(BIN_DIR)/rootfs/* $(BIN_DIR)/mnt/
	sudo umount $(BIN_DIR)/mnt
	@# Cleanup
	rm -rf $(BIN_DIR)/rootfs $(BIN_DIR)/mnt
	@echo "Rootfs created at $(BIN_DIR)/rootfs.ext4"

clean:
	rm -f $(BIN_DIR)/vmlinux $(BIN_DIR)/firecracker $(BIN_DIR)/rootfs.ext4
	rm -rf $(BIN_DIR)/rootfs $(BIN_DIR)/mnt