from fastapi import FastAPI, status
from fastapi.middleware.cors import CORSMiddleware
from app.config import config
from app.models.config import KeycloakConfig
from app.models.health import HealthCheck
from app.layers.views import router as layers_router
from app.styles.views import router as styles_router
from app.users.views import router as users_router
from app.countries.views import router as countries_router
from app.cogs.views import router as cogs_router

from rio_tiler.io import Reader
import attr
from rio_tiler.models import Info
from rasterio.io import MemoryFile
from rasterio import transform
import boto3
from rio_tiler.utils import (
    _validate_shape_input,
    has_alpha_band,
    has_mask_band,
)
from rasterio.vrt import WarpedVRT
import warnings
from rio_tiler.errors import (
    ExpressionMixingWarning,
    NoOverviewWarning,
    PointOutsideBounds,
    TileOutsideBounds,
)
import rasterio
from osgeo import gdal

from titiler.core.factory import TilerFactory
from titiler.core.errors import DEFAULT_STATUS_CODES, add_exception_handlers
from fastapi import Query, Depends
import aioboto3
from app.s3.services import get_s3

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


@attr.s
class S3Reader(Reader):
    """Override the Reader class to call S3 directly.

    Presigning URLs was not working.
    """

    def __attrs_post_init__(self):
        """Define _kwargs, open dataset and get info."""
        self.client = boto3.client(
            "s3",
            aws_access_key_id=config.S3_ACCESS_KEY,
            aws_secret_access_key=config.S3_SECRET_KEY,
            endpoint_url=f"https://{config.S3_URL}",
        )
        if not self.dataset:
            with MemoryFile(self._read(self.input)) as memfile:
                self.dataset = rasterio.open(memfile)

        self.bounds = tuple(self.dataset.bounds)
        self.crs = self.dataset.crs

        if self.colormap is None:
            self._get_colormap()

        if min(
            self.dataset.width, self.dataset.height
        ) > 512 and not self.dataset.overviews(1):
            warnings.warn(
                "The dataset has no Overviews. rio-tiler performances might be impacted.",
                NoOverviewWarning,
            )

    def _read(self, url: str):
        response = self.client.get_object(
            Bucket=config.S3_BUCKET_ID,
            Key=f"{config.S3_PREFIX}/{url}.tif",
        )
        return response["Body"].read()


cog = TilerFactory(reader=S3Reader)

app.include_router(
    cog.router,
    prefix=f"{config.API_PREFIX}/cog",
)

add_exception_handlers(app, DEFAULT_STATUS_CODES)
