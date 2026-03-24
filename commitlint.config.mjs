export default {
  extends: ["@commitlint/config-conventional"],
  rules: {
    // Allow proper nouns (React, TypeScript, etc.) in commit subjects
    "subject-case": [2, "never", ["start-case", "pascal-case", "upper-case"]],
  },
};
