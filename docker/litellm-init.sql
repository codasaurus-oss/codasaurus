CREATE DATABASE litellm;
SELECT 'CREATE DATABASE zitadel'
WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'zitadel')\gexec
