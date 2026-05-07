import Link from "next/link";

export const metadata = { title: "Not found" };

export default function NotFound() {
  return (
    <div className="proto-page-narrow">
      <h1>404</h1>
      <p className="proto-dek">
        That page slipped through the gravity well. Try the{" "}
        <Link href="/">front page</Link> or browse{" "}
        <Link href="/c">tags</Link>.
      </p>
    </div>
  );
}
