/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        surface: {
          0: "#09090b",
          1: "#0e0f13",
          2: "#15171e",
          3: "#1d202a",
        },
        border: {
          subtle: "rgba(255, 255, 255, 0.075)",
          strong: "rgba(255, 255, 255, 0.16)",
        },
        accent: {
          DEFAULT: "#6366f1",
          hover: "#818cf8",
        },
      },
      boxShadow: {
        facet: "inset 0 1px 0 rgba(255, 255, 255, 0.06), 0 10px 28px -16px rgba(0, 0, 0, 0.8)",
        keycap: "inset 0 1px 0 rgba(255, 255, 255, 0.1), 0 1.5px 0 rgba(0, 0, 0, 0.75)",
      },
      fontFamily: {
        sans: ["Inter", "-apple-system", "BlinkMacSystemFont", "Segoe UI", "sans-serif"],
        mono: ["JetBrains Mono", "Fira Code", "ui-monospace", "monospace"],
      },
    },
  },
  plugins: [],
};
