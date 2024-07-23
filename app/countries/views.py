from app.db import get_session, AsyncSession
from fastapi import Depends, APIRouter
from typing import Any
from sqlmodel import select
from app.countries.models import Country
from sqlalchemy import func
import json

router = APIRouter()


@router.get("")
async def get_all_countries(
    session: AsyncSession = Depends(get_session),
) -> Any:
    """Get all countries and export geometry in WKT"""

    # Return wkt of the WKBElement that is return in the geom column

    query = select(
        Country.name,
        Country.iso_a2,
        Country.iso_a3,
        Country.iso_n3,
        func.ST_AsGeoJSON(Country.geom).label("geom"),
    )

    result = await session.exec(query)
    countries = result.all()

    geojson_feature_collection = {
        "type": "FeatureCollection",
        "features": [
            {
                "type": "Feature",
                "properties": {
                    "name": country.name,
                    "iso_a2": country.iso_a2,
                    "iso_a3": country.iso_a3,
                    "iso_n3": country.iso_n3,
                },
                "geometry": json.loads(country.geom),
            }
            for country in countries
        ],
    }

    return geojson_feature_collection
