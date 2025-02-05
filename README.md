# Init

Init system for firecracker vm.

## Building the Project

To build the project, run:
```
make build
```

## Setting Up the VM for Testing

To run the executable in a VM for testing, follow these steps:

1. Download `firecracker`:
    ```
    make firecracker
    ```
2. Download the `linux kernel`:
    ```
    make kernel
    ```
   These commands will download Firecracker and the kernel to the `bin` directory.

3. Set up a filesystem for the device:
    ```
    make tmpinit
    ```

4. Set up a root filesystem:
    ```
    make rootfs DOCKER_IMAGE=your-docker-image-name
    ```

## Running the Init Binary in the VM

To run the init binary in the VM, execute:
```
make run
```