import NextAuth from "next-auth";
import Credentials from "next-auth/providers/credentials";
import bcrypt from "bcryptjs";
import { getDb, getAuthSecret } from "@/lib/server/db";
import { deriveKey } from "@/lib/server/crypto";

export const { handlers, auth, signIn, signOut } = NextAuth({
  secret: getAuthSecret(),
  session: { strategy: "jwt" },
  pages: { signIn: "/" },
  providers: [
    Credentials({
      credentials: {
        username: {},
        password: {},
      },
      async authorize(credentials) {
        const { username, password } = credentials as {
          username: string;
          password: string;
        };
        if (!username || !password) return null;

        const db = getDb();
        const user = db
          .prepare(
            "SELECT id, username, password_hash, key_salt FROM users WHERE username = ?"
          )
          .get(username) as
          | {
              id: string;
              username: string;
              password_hash: string;
              key_salt: string;
            }
          | undefined;

        if (!user || !bcrypt.compareSync(password, user.password_hash)) {
          return null;
        }

        // Derive AES key and cache in DB for wallet decryption
        const dk = deriveKey(password, user.key_salt);
        db.prepare("UPDATE users SET derived_key = ? WHERE id = ?").run(
          dk.toString("base64"),
          user.id
        );

        return { id: user.id, name: user.username };
      },
    }),
  ],
  callbacks: {
    jwt({ token, user }) {
      if (user) {
        token.userId = user.id;
      }
      return token;
    },
    session({ session, token }) {
      if (session.user) {
        session.user.id = token.userId as string;
      }
      return session;
    },
  },
});
