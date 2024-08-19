from pydantic import model_validator
from pydantic_settings import BaseSettings
from functools import lru_cache


class Config(BaseSettings):
    API_PREFIX: str = "/api"

    # Common Keycloaksettings
    KEYCLOAK_REALM: str
    KEYCLOAK_URL: str

    # Keycloak Admin settings
    KEYCLOAK_API_ID: str
    KEYCLOAK_API_SECRET: str

    # Keycloak UI settings
    KEYCLOAK_CLIENT_ID: str

    # PostGIS settings
    DB_HOST: str | None = None
    DB_PORT: int | None = None  # 5432
    DB_USER: str | None = None
    DB_PASSWORD: str | None = None

    DB_NAME: str | None = None  # postgres
    DB_PREFIX: str = "postgresql+asyncpg"

    DB_URL: str | None = None

    # S3 settings
    S3_BUCKET_ID: str
    S3_URL: str
    S3_ACCESS_KEY: str
    S3_SECRET_KEY: str
    S3_PREFIX: str

    # Redis cache
    TILE_CACHE_ENABLED: bool = True
    TILE_CACHE_URL: str
    TILE_CACHE_PORT: int
    TILE_CACHE_TTL: int = 3600

    @model_validator(mode="after")
    @classmethod
    def form_db_url(cls, values: dict) -> dict:
        """Form the DB URL from the settings"""
        if not values.DB_URL:
            values.DB_URL = (
                "{prefix}://{user}:{password}@{host}:{port}/{db}".format(
                    prefix=values.DB_PREFIX,
                    user=values.DB_USER,
                    password=values.DB_PASSWORD,
                    host=values.DB_HOST,
                    port=values.DB_PORT,
                    db=values.DB_NAME,
                )
            )
        return values


@lru_cache()
def get_config():
    return Config()


config = get_config()
