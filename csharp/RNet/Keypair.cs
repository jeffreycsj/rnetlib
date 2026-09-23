using System.Security.Cryptography;

namespace RNet;

/// <summary>X25519 identity. Dispose erases this private copy; callers must erase their exported copies.</summary>
public sealed unsafe class Keypair : IDisposable {
    private readonly object gate = new();
    private byte[]? privateKey;
    private readonly byte[] publicKey;
    public byte[] PublicKey => (byte[])publicKey.Clone();
    private Keypair(Native.Key key) {
        privateKey = new ReadOnlySpan<byte>(key.Private, 32).ToArray();
        publicKey = new ReadOnlySpan<byte>(key.Public, 32).ToArray();
        CryptographicOperations.ZeroMemory(new Span<byte>(key.Private, 32));
    }
    public static Keypair Generate() {
        Native.Key key = default;
        try { Native.Check(Native.rnet_keypair_generate(&key)); return new Keypair(key); }
        finally { CryptographicOperations.ZeroMemory(new Span<byte>(key.Private, 32)); }
    }
    public static Keypair FromPrivate(ReadOnlySpan<byte> bytes) {
        Native.Key key = default;
        fixed (byte* ptr = bytes) {
            try { Native.Check(Native.rnet_keypair_from_private(new Native.Slice(ptr, bytes.Length), &key)); return new Keypair(key); }
            finally { CryptographicOperations.ZeroMemory(new Span<byte>(key.Private, 32)); }
        }
    }
    public byte[] ExportPrivateKey() {
        lock (gate) { ObjectDisposedException.ThrowIf(privateKey == null, this); return (byte[])privateKey!.Clone(); }
    }
    public void Dispose() {
        lock (gate) { if (privateKey != null) CryptographicOperations.ZeroMemory(privateKey); privateKey = null; }
        GC.SuppressFinalize(this);
    }
    ~Keypair() { Dispose(); }
    public override string ToString() => "Keypair(<redacted>)";
}
