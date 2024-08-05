from fastapi import FastAPI, status
from fastapi.middleware.cors import CORSMiddleware
from app.config import config
from app.models.config import KeycloakConfig
from app.models.health import HealthCheck
from app.layers.views import router as layers_router
from app.styles.views import router as styles_router
from app.users.views import router as users_router
from app.countries.views import router as countries_router
from rio_tiler.io import Reader
import attr
from rasterio.io import MemoryFile
import boto3
import warnings
from rio_tiler.errors import NoOverviewWarning
import rasterio
from titiler.core.factory import TilerFactory
from titiler.core.errors import DEFAULT_STATUS_CODES, add_exception_handlers
from fastapi import Query, Depends
import aioboto3
from app.s3.services import get_s3
from app.db import get_session, AsyncSession
from sqlmodel import select
from app.layers.models import Layer


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


import json

from typing import Dict, Optional, Literal
from typing_extensions import Annotated


from rio_tiler.colormap import parse_color
from rio_tiler.colormap import cmap as default_cmap
from fastapi import HTTPException, Query

from fastapi import Depends, Query, HTTPException
from typing import Optional, Dict
from sqlmodel import select
from app.db import get_session, AsyncSession
from app.s3.services import get_s3
import aioboto3


async def ColorMapParams(
    s3: aioboto3.Session = Depends(get_s3),
    session: AsyncSession = Depends(get_session),
    url: str = Query(),
) -> Optional[Dict]:
    """Colormap Dependency."""

    query = select(Layer).where(Layer.layer_name == url)
    layer = await session.exec(query)
    layer = layer.one_or_none()

    if not layer:
        raise HTTPException(status_code=404, detail="Layer not found")

    if layer.style and layer.style.style:
        style = layer.style.style
        print(f"Using style from DB for {url}: {style}")

        # Sort the style array by the 'value' field
        style_sorted = sorted(style, key=lambda x: x["value"])

        # Create the colormap with end values being the start of the next
        colormap = []
        for i in range(len(style_sorted)):
            start = style_sorted[i]["value"]
            end = (
                style_sorted[i + 1]["value"]
                if i + 1 < len(style_sorted)
                else start + 1.0
            )  # or some logical end value
            color = [
                style_sorted[i]["red"],
                style_sorted[i]["green"],
                style_sorted[i]["blue"],
                style_sorted[i]["opacity"],
            ]
            colormap.append([[start, end], color])

        print("USING COLORMAP:", colormap)
        return colormap

    min_value = layer.min_value
    max_value = layer.max_value

    num_segments = 10
    colors = [
        [0, 0, 0, 0],
        [215, 25, 28, 255],
        [253, 174, 97, 255],
        [255, 255, 191, 255],
        [171, 221, 164, 255],
        [43, 131, 186, 255],
        [0, 104, 55, 255],
        [166, 217, 106, 255],
        [255, 255, 204, 255],
        [253, 141, 60, 255],
    ]

    step = (max_value - min_value) / num_segments

    colormap = []
    for i in range(num_segments):
        start = min_value + i * step
        end = min_value + (i + 1) * step
        color = colors[i % len(colors)]
        colormap.append([[start, end], color])

    return colormap


cog = TilerFactory(
    reader=S3Reader,
    colormap_dependency=ColorMapParams,
)

app.include_router(
    cog.router,
    prefix=f"{config.API_PREFIX}/cog",
)

add_exception_handlers(app, DEFAULT_STATUS_CODES)
