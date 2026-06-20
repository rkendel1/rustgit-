const API_BASE_URL =
  process.env.NEXT_PUBLIC_API_URL ?? "https://api.trythissoftware.com";

export default function Home() {
  return (
    <main
      style={{
        minHeight: "100vh",
        display: "grid",
        placeItems: "center",
        padding: "2rem 1rem",
      }}
    >
      <section
        style={{
          width: "100%",
          maxWidth: "42rem",
          border: "1px solid #dbeafe",
          borderRadius: "0.75rem",
          padding: "1.5rem",
          background: "#ffffff",
        }}
      >
        <h1 style={{ marginBottom: "0.75rem" }}>TryThisSoftware Portal</h1>
        <p style={{ marginBottom: "0.75rem" }}>
          Welcome to the deployed portal home page.
        </p>
        <p style={{ marginBottom: "0.75rem" }}>
          API base: <code>{API_BASE_URL}</code>
        </p>
        <p>
          Health check: <a href="/api/health">/api/health</a>
        </p>
      </section>
    </main>
  );
}
