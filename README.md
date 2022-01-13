# helios-omicron-brand

This repository contains the `omicron1` zone brand for Helios.

<!-- MANUAL START -->
```
OMICRON1(5)        Standards, Environments, and Macros       OMICRON1(5)

NAME
     omicron1 – zone brand for running Omicron components

DESCRIPTION
     The omicron1 brand uses the brands(5) framework to provide an
     environment in which to run Omicron components within the Oxide
     Helios ramdisk image.  Each zone is created from the combination of
     the libraries and executables on the running system, a baseline
     archive of pristine configuration files, and an optional set of
     image archives provided at install time.

     This brand is intended for use in a ramdisk system where zones are
     not persistent; they are recreated from images as needed each time
     the system boots.  If persistent data is required, such as for a
     database server, the zone should be provided with a delegated ZFS
     dataset and the application should be configured to store its data
     within that dataset.  See DELEGATING A DATASET.

BASELINE ARCHIVES
   Production Systems
     In production systems the baseline archive will be generated
     against the ramdisk contents at build time and shipped as a file in
     the ramdisk itself.  This work is not yet complete.

   Development Systems
     For development purposes, a baseline archive can be generated
     against the running system.  An SMF service,
     svc:/system/omicron/baseline:default, is provided in the
     pkg:/system/zones/brand/omicron1/tools package, and will generate a
     baseline archive at boot and store it in /var/run/brand/omicron
     where the brand will look for it when installing a zone with
     zoneadm(1M).

     The baseline generator makes use of pkg(5) to assemble the contents
     of the zone root file system into an archive that can be unpacked
     for each install.  Once the archive is generated, whether on the
     development system or in the ramdisk, zones can be installed
     without further interaction with the packaging system.  Any time
     pkg(1) is used to update a development system, the contents of any
     previously installed zones is likely invalid and they will need to
     be uninstalled and reinstalled.  See LIMITATIONS.

IMAGE ARCHIVES
     An image archive is a gzip-compressed tar file with a specific
     layout.  The first file in the archive should be a file with the
     name oxide.json and the following contents:

       {"v":"1","t":"layer"}

     This metadata is used by the brand to identify the type of image so
     that it may be unpacked correctly.

     Files to be unpacked into the zone root must be stored within the
     archive under a directory called root.  A minimal image might
     contain very few files; e.g.,

       $ tar tfz /tmp/someimage.tar.gz
       oxide.json
       root/
       root/var/svc/manifest/site/program1.xml
       root/var/svc/manifest/site/program2.xml
       root/var/svc/profile/site.xml
       root/usr/lib/program1
       root/usr/lib/program2

CONFIGURATION
     Zones using the omicron1 brand must have an exclusive IP stack.  If
     networking in the zone is required, create a VNIC with dladm(1M)
     (e.g., testzone0 below) and pass it in the zone configuration.
     There is presently no mechanism for automated IP configuration from
     zone properties, though enabling the svc:/network/physical:nwam
     service within the zone will make a best effort attempt at IPv4 and
     IPv6 automated configuration.

       create
       set brand=omicron1
       set zonepath=/zones/testzone
       set autoboot=false
       set ip-type=exclusive
       add net
           set physical=testzone0
       end

INSTALLATION
     At zone install time, the omicron1 brand unpacks the baseline
     archive into the zone root to establish a basic Helios environment
     inside the zone.  Additional files may optionally be layered on top
     of the base system by providing them as arguments to zoneadm(1M)
     when installing the zone:

       # zoneadm -z testzone0 install /tmp/someimage.tar.gz
       A ZFS file system has been created for this zone.
       INFO: omicron: installing zone testzone @ "/zones/testzone"...
       INFO: omicron: replicating /usr tree...
       INFO: omicron: replicating /lib tree...
       INFO: omicron: replicating /sbin tree...
       INFO: omicron: pruning SMF manifests...
       INFO: omicron: pruning global-only files...
       INFO: omicron: unpacking baseline archive...
       INFO: omicron: unpacking image "/tmp/someimage.tar.gz"...
       INFO: omicron: install complete, probably!

     Note that, as per LIMITATIONS, for correct operation each omicron1
     zone must be uninstalled and reinstalled any time a development
     system is updated, such as with pkg(1).  In a production ramdisk
     system, this will effectively happen automatically as all zone
     state is discarded each time the system reboots.

DELEGATING A DATASET
     One can create a delegated dataset that will automatically mount at
     /data within the zone as follows:

       # zfs create -o canmount=noauto rpool/delegated/testzone
       # zfs set mountpoint=/data rpool/delegated/testzone
       # zfs set zoned=on rpool/delegated/testzone
       # zfs set canmount=on rpool/delegated/testzone

     Once this dataset has been created, it can be delegated to the zone
     via zonecfg(1M):

       # zonecfg -z testzone
       zonecfg:testzone> add dataset
       zonecfg:testzone:dataset> set name=rpool/delegated/testzone
       zonecfg:testzone:dataset> end
       zonecfg:testzone> commit

     The dataset will then be mounted at /data next time the zone boots.
     Note that the delegated dataset can also be added during initial
     zone configuration, and does not need to be added as a second step.

LIMITATIONS
     Various components in the operating system share Private interfaces
     with one another; e.g., the system call interface used by libc.so.1
     and the kernel are subject to change without notice.  To ensure
     correct operation such components must always be built together,
     and installed or updated in lock step.

     During zone installation, the omicron1 brand draws baseline files,
     which include such components with Private interfaces, from two
     sources:

     -   Executable and library files in /usr, /lib, and /sbin, which
         are copied or symlinked from their source locations on the
         running system.

     -   Seed configuration and data files, such as those in /etc and
         /var, which are unpacked from the baseline archive.

     The copying and symlinking of contents from trees like /usr
     represents a compromise:

     -   It allows us to modify those trees by layering on additional
         files from images, which a read-only lofs(7FS) mount would not.
         A more complete solution for the future, which would require
         more engineering effort, would be to develop a union or layered
         file system.

     -   It requires us to reinstall the zone any time the contents of
         /usr, /lib, or /sbin, change; i.e., any time the operating
         system is updated.  This is only a problem on development
         machines, as production ramdisks are sealed at build time and
         zones will be recreated each time the machine boots.

     In short: use zoneadm(1M) to uninstall and reinstall your omicron1
     brand zones after updating with pkg(1) and rebooting.

INTERFACE STABILITY
     During early development the brand will continue to evolve, and is
     thus Uncommitted.

SEE ALSO
     pkg(1), dladm(1M), zfs(1M), zoneadm(1M), zonecfg(1M), brands(5),
     pkg(5), zones(5), lofs(7FS)

illumos                     December 12, 2021                    illumos
```
<!-- MANUAL END -->
