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
# app.include_router(
#     cogs_router,
#     prefix=f"{config.API_PREFIX}/cogs",
#     tags=["cogs"],
# )

from titiler.core.factory import TilerFactory
from titiler.core.errors import DEFAULT_STATUS_CODES, add_exception_handlers
from fastapi import Query, Depends
import aioboto3
from app.s3.services import get_s3


# Custom Path dependency which will sign url
# !!! You may want to add caching here to avoid to many call to the signing provider !!!
async def DatasetPathParams(
    url: str = Query(..., description="Layer name"),
    s3: aioboto3.Session = Depends(get_s3),
) -> str:
    """Create dataset path from args"""

    signed_url = await s3.generate_presigned_url(
        ClientMethod="get_object",
        Params={
            "Bucket": config.S3_BUCKET_ID,
            "Key": f"{config.S3_PREFIX}/{url}.tif",
            # "ResponseContentDisposition": 'attachment; filename="your-download-filename"',
            "ResponseContentType": "application/octet-stream",
        },
        HttpMethod="GET",
        ExpiresIn=36000,
    )
    import urllib

    print(f"Signed URL: {signed_url}")
    print(f"Signed URL: {urllib.parse.unquote(signed_url)}")
    print(f"Type of signed URL: {type(signed_url)}")

    # # Use httpx to get data from the signed URL
    # import httpx

    # async with httpx.AsyncClient() as client:

    #     response = await client.get(signed_url)
    #     print(f"Response: {response}")
    #     print(f"Type of response: {type(response)}")
    #     # print(f"Response content: {response.content}")
    #     print(f"Type of response content: {type(response.content)}")

    # # Use your provider library to sign the URL
    url, params = signed_url.split("?")
    return url
    return urllib.parse.unquote(signed_url)


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
                # with memfile.open() as dataset:
                # self.dataset = self._ctx_stack.enter_context(
                # rasterio.open(dataset.read())
                # )

                # with memfile.open() as dataset:
                #     self.dataset = dataset.open()
        print(f"Dataset: {self.dataset}")
        print(f"Shape of dataset: {self.dataset.shape}")
        # self.dataset = rasterio.open(self._read(self.input))

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

    # def __attrs_post_init__(self):
    #     self.client = boto3.client(
    #         "s3",
    #         aws_access_key_id=config.S3_ACCESS_KEY,
    #         aws_secret_access_key=config.S3_SECRET_KEY,
    #         endpoint_url=f"https://{config.S3_URL}",
    #     )

    #     data = self._read(self.input)
    #     self.dataset = MemoryFile(data).open()

    #     self.bounds = tuple(self.dataset.bounds)
    #     self.crs = self.dataset.crs

    #     if min(
    #         self.dataset.width, self.dataset.height
    #     ) > 512 and not self.dataset.overviews(1):
    #         warnings.warn(
    #             "The dataset has no Overviews. rio-tiler performances might be impacted.",
    #             NoOverviewWarning,
    #         )

    def _read(self, url: str):
        response = self.client.get_object(
            Bucket=config.S3_BUCKET_ID,
            Key=f"{config.S3_PREFIX}/{url}.tif",
        )
        print(response)
        return response["Body"].read()

    # def read(self, *args, **kwargs):
    #     """Override read method to fetch data from S3."""
    #     return self.dataset.read(*args, **kwargs)

    # def info(self):
    #     """Override info method to fetch metadata from S3."""
    #     info = {
    #         "bounds": self.dataset.bounds,
    #         "minzoom": 0,
    #         "maxzoom": 22,
    #         "band_metadata": [],
    #         "band_descriptions": [],
    #         "nodata_type": (
    #             "Nodata" if self.dataset.nodata is not None else "None"
    #         ),
    #         "dtype": str(self.dataset.dtypes[0]),
    #         "colorinterp": [
    #             str(interp) for interp in self.dataset.colorinterp
    #         ],
    #         "nodata_value": self.dataset.nodata,
    #         "driver": self.dataset.driver,
    #     }
    #     return Info(**info)


cog = TilerFactory(reader=S3Reader)

app.include_router(
    cog.router,
    prefix=f"{config.API_PREFIX}/cog",
)

add_exception_handlers(app, DEFAULT_STATUS_CODES)
