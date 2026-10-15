using System.Security.Cryptography;

namespace RNet;

/// <summary>X25519 identity. Dispose erases this private copy; callers must erase their exported copies.</summary>
public sealed unsafe class Keypair : IDisposable {
    private readonly object gate = new();
    private byte[]? privateKey;
    private readonly byte[] publicKey;
    /// <summary>Returns an independent 32-byte public-key copy; remains available after Dispose.</summary>
    public byte[] PublicKey => (byte[])publicKey.Clone();
    private Keypair(Native.Key key) {
        privateKey = new ReadOnlySpan<byte>(key.Private, 32).ToArray();
        publicKey = new ReadOnlySpan<byte>(key.Public, 32).ToArray();
        CryptographicOperations.ZeroMemory(new Span<byte>(key.Private, 32));
    }
    /// <summary>Generates a fresh identity using the current native ABI.</summary>
    /// <returns>Caller-owned keypair; dispose it to erase its managed private-key copy.</returns>
    public static Keypair Generate() {
        Native.CheckAbi();
        Native.Key key = default;
        try { Native.Check(Native.rnet_keypair_generate(&key)); return new Keypair(key); }
        finally { CryptographicOperations.ZeroMemory(new Span<byte>(key.Private, 32)); }
    }
    /// <summary>Copies an existing identity and derives its public key.</summary>
    /// <param name="bytes">Exactly 32 private-key bytes. Caller retains ownership and must erase its own copy.</param>
    /// <returns>Caller-owned keypair; invalid length throws RNetException.</returns>
    public static Keypair FromPrivate(ReadOnlySpan<byte> bytes) {
        Native.CheckAbi();
        Native.Key key = default;
        fixed (byte* ptr = bytes) {
            try { Native.Check(Native.rnet_keypair_from_private(new Native.Slice(ptr, bytes.Length), &key)); return new Keypair(key); }
            finally { CryptographicOperations.ZeroMemory(new Span<byte>(key.Private, 32)); }
        }
    }
    /// <summary>Copies the private key under the disposal lock.</summary>
    /// <returns>New secret 32-byte array; caller must erase it.</returns>
    /// <exception cref="ObjectDisposedException">The keypair was disposed.</exception>
    public byte[] ExportPrivateKey() {
        lock (gate) { ObjectDisposedException.ThrowIf(privateKey == null, this); return (byte[])privateKey!.Clone(); }
    }
    /// <summary>Erases this object's private array under a lock; repeated calls are harmless.</summary>
    /// <remarks>Exported copies are not erased. The public key remains readable.</remarks>
    public void Dispose() {
        lock (gate) { if (privateKey != null) CryptographicOperations.ZeroMemory(privateKey); privateKey = null; }
        GC.SuppressFinalize(this);
    }
    /// <summary>Fallback secret erasure; use Dispose for deterministic lifetime control.</summary>
    ~Keypair() { Dispose(); }
    /// <summary>Returns a redacted diagnostic without exposing public or private key bytes.</summary>
    public override string ToString() => "Keypair(<redacted>)";
}
