import argparse
import json
import os
import socket
import struct


def recv_exact(connection: socket.socket, size: int) -> bytes:
    chunks = bytearray()
    while len(chunks) < size:
        chunk = connection.recv(size - len(chunks))
        if not chunk:
            raise RuntimeError("RCON closed the connection.")
        chunks.extend(chunk)
    return bytes(chunks)


def send_packet(
    connection: socket.socket,
    packet_id: int,
    packet_type: int,
    body: str,
) -> None:
    body_bytes = body.encode("utf-8")
    size = 4 + 4 + len(body_bytes) + 2
    connection.sendall(
        struct.pack("<iii", size, packet_id, packet_type)
        + body_bytes
        + b"\x00\x00"
    )


def read_packet(connection: socket.socket) -> tuple[int, int, str]:
    size = struct.unpack("<i", recv_exact(connection, 4))[0]
    if size < 10 or size > 8192:
        raise RuntimeError("RCON returned an invalid packet size.")
    packet = recv_exact(connection, size)
    packet_id, packet_type = struct.unpack("<ii", packet[:8])
    body = packet[8:-2].decode("utf-8", errors="replace")
    return packet_id, packet_type, body


def execute(host: str, port: int, password: str, command: str) -> str:
    with socket.create_connection((host, port), timeout=6) as connection:
        connection.settimeout(6)
        send_packet(connection, 1, 3, password)
        authenticated = False
        for _ in range(2):
            packet_id, _, _ = read_packet(connection)
            if packet_id == -1:
                raise RuntimeError("RCON authentication failed.")
            if packet_id == 1:
                authenticated = True
                break
        if not authenticated:
            raise RuntimeError("RCON did not confirm authentication.")

        send_packet(connection, 2, 2, command)
        _, _, response = read_packet(connection)
        return response


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", required=True)
    parser.add_argument("--port", required=True, type=int)
    parser.add_argument("--command", required=True)
    args = parser.parse_args()

    password = os.environ.get("PALCMD_RCON_PASSWORD", "")
    if not password:
        raise RuntimeError("RCON password was not supplied.")
    response = execute(args.host, args.port, password, args.command)
    print(json.dumps({"response": response}))


if __name__ == "__main__":
    main()
