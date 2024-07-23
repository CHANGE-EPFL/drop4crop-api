from fastapi import FastAPI, status
from fastapi.middleware.cors import CORSMiddleware
from app.config import config
from app.models.config import KeycloakConfig
from app.models.health import HealthCheck
from app.layers.views import router as layers_router
from app.styles.views import router as styles_router
from app.users.views import router as users_router
from app.countries.views import router as countries_router

app = FastAPI()

origins = ["*"]

app.add_middleware(
    CORSMiddleware,
    allow_origins=origins,
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


@app.get(f"{config.API_PREFIX}/config/keycloak")
async def get_keycloak_config() -> KeycloakConfig:
    return KeycloakConfig(
        clientId=config.KEYCLOAK_CLIENT_ID,
        realm=config.KEYCLOAK_REALM,
        url=config.KEYCLOAK_URL,
    )


@app.get(
    f"{config.API_PREFIX}/healthz",
    tags=["healthcheck"],
    summary="Perform a Health Check",
    response_description="Return HTTP Status Code 200 (OK)",
    status_code=status.HTTP_200_OK,
    response_model=HealthCheck,
)
def get_health() -> HealthCheck:
    """Perform a Health Check

    Useful for Kubernetes to check liveness and readiness probes
    """
    return HealthCheck(status="OK")


@app.get(f"{config.API_PREFIX}/config/geoserver", response_model=str)
def get_geoserver_config() -> str:
    return config.GEOSERVER_URL + "/drop4crop/wms"


app.include_router(
    layers_router,
    prefix=f"{config.API_PREFIX}/layers",
    tags=["layers"],
)
app.include_router(
    styles_router,
    prefix=f"{config.API_PREFIX}/styles",
    tags=["styles"],
)
app.include_router(
    users_router,
    prefix=f"{config.API_PREFIX}/users",
    tags=["users"],
)
app.include_router(
    countries_router,
    prefix=f"{config.API_PREFIX}/countries",
    tags=["countries"],
)
