https://gstreamer.freedesktop.org/download/#android

```console
adb forward tcp:30069 tcp:30069
```

```console
telnet localhost 5554
# auth stuff
redir add udp:5004:5004
```
